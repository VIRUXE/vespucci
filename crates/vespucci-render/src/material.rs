//! Binding a drawable's material (`ShaderFx`) to a technique of its `.fxc`:
//! constant buffers from reflection, defaults from the `.fxc`, values from
//! the material, textures by name.

use crate::cbuffer::CBufferBlock;
use crate::shader_cache::{PixelStage, VertexStage};
use crate::texture::TextureCache;
use anyhow::Result;
use glam::Mat4;
use rage_formats::{ShaderFx, ShaderParameterValue, YtdTexture};
use std::collections::HashMap;
use std::rc::Rc;
use vespucci_d3d11::ffi::*;
use vespucci_d3d11::{ComPtr, Device, Stage};
use vespucci_fxc::dxbc::BindKind;
use vespucci_fxc::FxcFile;
use vespucci_game::joaat;

/// Constant buffers the engine fills, not the material.
pub const GLOBAL_CBUFFERS: [&str; 12] = [
    "rage_matrices",
    "rage_bonemtx",
    "rage_clipplanes",
    "misc_globals",
    "lighting_globals",
    "more_stuff",
    "warpshadow",
    "csmshader",
    "rage_cbinst_matrices",
    "rage_cbinst_update",
    "gpu_instancing_globals",
    "rage_lodmtx",
];

pub fn is_global(name: &str) -> bool {
    GLOBAL_CBUFFERS.iter().any(|g| g.eq_ignore_ascii_case(name))
}

/// One engine-owned constant buffer shared by every material that binds it.
pub struct Globals {
    pub blocks: HashMap<String, CBufferBlock>,
    /// Per-frame values some shaders keep in their own (material-owned) cbuffer
    /// rather than in a global one, written into every material that declares them at bind time.
    pub frame: FrameParams,
}

/// Engine values that live in material cbuffers (see [`Material::bind`]).
#[derive(Debug, Clone, Default)]
pub struct FrameParams {
    /// `gViewProj`: view-projection in the same packing as `rage_matrices` (the cable shader
    /// projects with this instead of `gWorldViewProj`).
    pub view_proj: [f32; 16],
    /// `gCableParams`: x = pixels per metre at 1 m (half the viewport height over tan(fov/2)),
    /// y = radius scale, z = fade-exponent scale, w = alpha scale.
    pub cable_params: [f32; 4],
}

/// Which per-frame value a material cbuffer variable takes.
#[derive(Debug, Clone, Copy)]
pub enum FrameVar {
    ViewProj,
    CableParams,
}

impl Globals {
    pub fn new() -> Globals {
        Globals {
            blocks: HashMap::new(),
            frame: FrameParams {
                view_proj: Mat4::IDENTITY.to_cols_array(),
                cable_params: [1.0, 1.0, 1.0, 1.0],
            },
        }
    }

    /// The block for a global cbuffer, created from this shader's reflection
    /// the first time it is seen, with the `.fxc` defaults written in.
    pub fn block(
        &mut self,
        dev: &Device,
        cb: &vespucci_fxc::dxbc::RdefCBuffer,
        fxc: &FxcFile,
    ) -> Result<&mut CBufferBlock> {
        let key = cb.name.to_lowercase();
        if !self.blocks.contains_key(&key) {
            let mut block = CBufferBlock::new(dev, cb)?;
            apply_fxc_defaults(&mut block, fxc, cb.name.as_str());
            self.blocks.insert(key.clone(), block);
        }
        Ok(self.blocks.get_mut(&key).unwrap())
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut CBufferBlock> {
        self.blocks.get_mut(&name.to_lowercase())
    }

    pub fn upload_all(&mut self, dev: &Device) -> Result<()> {
        for b in self.blocks.values_mut() {
            b.upload(dev)?;
        }
        Ok(())
    }
}

impl Default for Globals {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_fxc_defaults(block: &mut CBufferBlock, fxc: &FxcFile, cbuffer_name: &str) {
    let hash = joaat(&cbuffer_name.to_lowercase());
    for v in fxc
        .variables
        .iter()
        .filter(|v| v.cbuffer_hash == hash && !v.values.is_empty())
    {
        let floats = v.default_floats();
        if !block.set_f32(joaat(&v.name), &floats) {
            block.set_f32(joaat(&v.name.to_lowercase()), &floats);
        }
    }
}

/// The sampler states a draw uses until the `.fxc` sampler states are applied (#6):
/// one filtered wrap sampler for textures, the comparison sampler for shadow maps,
/// and point + clamp for tint palettes (one colour per texel; wrap blends the real
/// colour in column 0 with the pink filler in column 255).
pub struct Samplers {
    pub default: ComPtr<ID3D11SamplerState>,
    pub comparison: ComPtr<ID3D11SamplerState>,
    pub palette: ComPtr<ID3D11SamplerState>,
}

impl Samplers {
    pub fn new(dev: &Device) -> Result<Samplers> {
        Ok(Samplers {
            default: dev.create_sampler(
                D3D11_FILTER_ANISOTROPIC,
                D3D11_TEXTURE_ADDRESS_WRAP,
                16,
            )?,
            comparison: dev.create_comparison_sampler()?,
            palette: dev.create_sampler(
                D3D11_FILTER_MIN_MAG_MIP_POINT,
                D3D11_TEXTURE_ADDRESS_CLAMP,
                1,
            )?,
        })
    }

    /// The sampler for a binding, by its name in the shader.
    pub fn for_binding(&self, name: &str) -> &ComPtr<ID3D11SamplerState> {
        let lower = name.to_lowercase();
        if lower.contains("shadow") {
            &self.comparison
        } else if lower.contains("tintpalette") {
            &self.palette
        } else {
            &self.default
        }
    }
}

pub struct BoundCBuffer {
    pub stage: Stage,
    pub slot: u32,
    pub global: Option<String>,
    pub block: Option<CBufferBlock>,
}

pub struct BoundTexture {
    pub stage: Stage,
    pub slot: u32,
    pub name: String,
    pub texture_name: Option<String>,
    pub srv: ComPtr<ID3D11ShaderResourceView>,
    pub found: bool,
    /// No material parameter names this input: the engine supplies it per frame.
    pub engine: bool,
}

pub struct Material {
    pub shader_name: String,
    pub technique: String,
    pub vs: Rc<VertexStage>,
    pub ps: Rc<PixelStage>,
    pub cbuffers: Vec<BoundCBuffer>,
    pub textures: Vec<BoundTexture>,
    /// (stage, slot, binding name); the name picks the state, see [`Samplers::for_binding`].
    pub samplers: Vec<(Stage, u32, String)>,
    pub params_applied: usize,
    pub params_unmatched: Vec<String>,
    /// The drawable's render bucket: 0 opaque, 1 alpha, 2 decal, 3 cutout.
    pub bucket: u8,
    /// Material cbuffer variables the engine fills per frame: (cbuffer index, name hash, value).
    pub frame_vars: Vec<(usize, u32, FrameVar)>,
    /// `_tnt` shaders: (cbuffer index, hash of `tintPaletteSelector`, palette rows), so the
    /// entity's tint picks a palette row per draw ([`Material::set_tint`]).
    pub tint_var: Option<(usize, u32, u32)>,
}

impl Material {
    /// Binds `fx` (a drawable's material) to the compiled stages of its shader.
    /// `find_texture` resolves a texture name hash to a game texture.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        dev: &Device,
        fxc: &FxcFile,
        shader_name: &str,
        technique: &str,
        vs: Rc<VertexStage>,
        ps: Rc<PixelStage>,
        fx: &ShaderFx,
        globals: &mut Globals,
        textures: &mut TextureCache,
        mut find_texture: impl FnMut(u32) -> Option<YtdTexture>,
        mip_skip: u8,
    ) -> Result<Material> {
        let mut cbuffers = Vec::new();
        let mut params_applied = 0;
        let mut params_unmatched = Vec::new();

        for (stage, rdef) in [
            (Stage::Vertex, &vs.stage.rdef),
            (Stage::Pixel, &ps.stage.rdef),
        ] {
            for b in rdef.bindings.iter().filter(|b| b.kind == BindKind::CBuffer) {
                let Some(cb) = rdef.cbuffers.iter().find(|c| c.name == b.name) else {
                    continue;
                };
                if is_global(&cb.name) {
                    globals.block(dev, cb, fxc)?;
                    cbuffers.push(BoundCBuffer {
                        stage,
                        slot: b.bind_point,
                        global: Some(cb.name.to_lowercase()),
                        block: None,
                    });
                } else {
                    let mut block = CBufferBlock::new(dev, cb)?;
                    apply_fxc_defaults(&mut block, fxc, &cb.name);
                    for p in &fx.parameters {
                        if let ShaderParameterValue::Vectors(v) = &p.value {
                            let floats: Vec<f32> =
                                v.iter().flat_map(|x| [x.x, x.y, x.z, x.w]).collect();
                            if block.set_f32(p.name_hash, &floats) {
                                params_applied += 1;
                                log::trace!(
                                    "{shader_name}: {}:{} = {:?}",
                                    block.name,
                                    block.name_of(p.name_hash).unwrap_or("?"),
                                    &floats[..floats.len().min(8)]
                                );
                            }
                        }
                    }
                    cbuffers.push(BoundCBuffer {
                        stage,
                        slot: b.bind_point,
                        global: None,
                        block: Some(block),
                    });
                }
            }
        }
        // Debug: VESPUCCI_SET_VAR=cbuffer:var=a,b,c,d;... overrides material variables everywhere.
        if let Ok(spec) = std::env::var("VESPUCCI_SET_VAR") {
            for item in spec.split(';') {
                if let Some((target, values)) = item.split_once('=') {
                    if let Some((cb, var)) = target.split_once(':') {
                        let v: Vec<f32> = values
                            .split(',')
                            .filter_map(|x| x.trim().parse().ok())
                            .collect();
                        for c in &mut cbuffers {
                            if let Some(b) = &mut c.block {
                                if b.name == cb {
                                    b.set_by_name(var, &v);
                                }
                            }
                        }
                    }
                }
            }
        }
        for p in &fx.parameters {
            if let ShaderParameterValue::Vectors(_) = &p.value {
                let matched = cbuffers
                    .iter()
                    .any(|c| c.block.as_ref().is_some_and(|b| b.has(p.name_hash)));
                if !matched {
                    params_unmatched.push(format!("{:#010x}", p.name_hash));
                }
            }
        }

        let mut bound_textures = Vec::new();
        let mut samplers = Vec::new();
        let mut palette_rows = None;
        for (stage, rdef) in [
            (Stage::Vertex, &vs.stage.rdef),
            (Stage::Pixel, &ps.stage.rdef),
        ] {
            for b in &rdef.bindings {
                match b.kind {
                    BindKind::Sampler => samplers.push((stage, b.bind_point, b.name.clone())),
                    BindKind::Texture => {
                        // The material parameter of the same name says which texture.
                        let (h1, h2) = (joaat(&b.name), joaat(&b.name.to_lowercase()));
                        let param = fx
                            .parameters
                            .iter()
                            .find(|p| p.name_hash == h1 || p.name_hash == h2);
                        let mut texture_name = None;
                        let is_shadow = b.name.to_lowercase().contains("shadow");
                        let engine = param.is_none() && !is_shadow;
                        let mut srv = if is_shadow {
                            textures.unshadowed.clone_ptr()
                        } else if engine {
                            textures.engine.clone_ptr()
                        } else {
                            textures.missing.clone_ptr()
                        };
                        let mut found = is_shadow;
                        if let Some(p) = param {
                            if let ShaderParameterValue::Texture { name, name_hash } = &p.value {
                                texture_name = Some(name.clone());
                                if let Some(existing) = textures.get(*name_hash) {
                                    srv = existing.clone_ptr();
                                    found = true;
                                } else if let Some(t) = find_texture(*name_hash) {
                                    match textures.insert(dev, &t, mip_skip) {
                                        Ok(s) => {
                                            srv = s.clone_ptr();
                                            found = true;
                                        }
                                        Err(e) => log::warn!("{name}: {e:#}"),
                                    }
                                }
                                if found && b.name.eq_ignore_ascii_case("TintPaletteSampler") {
                                    palette_rows = textures.size(*name_hash).map(|(_, h)| h);
                                }
                            }
                        }
                        bound_textures.push(BoundTexture {
                            stage,
                            slot: b.bind_point,
                            name: b.name.clone(),
                            texture_name,
                            srv,
                            found,
                            engine,
                        });
                    }
                    _ => {}
                }
            }
        }

        // Per-frame engine values that some shaders keep in their material cbuffer.
        let mut frame_vars = Vec::new();
        for (i, c) in cbuffers.iter().enumerate() {
            let (None, Some(b)) = (&c.global, &c.block) else {
                continue;
            };
            for (name, var) in [
                ("gViewProj", FrameVar::ViewProj),
                ("gCableParams", FrameVar::CableParams),
            ] {
                let h = joaat(name);
                if b.has(h) {
                    frame_vars.push((i, h, var));
                }
            }
        }

        // The palette row selector, when the shader has one and the palette was found.
        let selector = joaat("tintPaletteSelector");
        let tint_var = palette_rows.and_then(|rows| {
            cbuffers.iter().enumerate().find_map(|(i, c)| {
                c.block
                    .as_ref()
                    .is_some_and(|b| c.global.is_none() && b.has(selector))
                    .then_some((i, selector, rows))
            })
        });
        if let Some((i, _, rows)) = tint_var {
            log::debug!(
                "{shader_name}: tint palette with {rows} rows, selector in {}",
                cbuffers[i].block.as_ref().map_or("?", |b| b.name.as_str())
            );
        }

        Ok(Material {
            shader_name: shader_name.to_string(),
            technique: technique.to_string(),
            vs,
            ps,
            cbuffers,
            textures: bound_textures,
            samplers,
            params_applied,
            params_unmatched,
            bucket: fx.render_bucket,
            frame_vars,
            tint_var,
        })
    }

    /// Selects the palette row for an entity's tint: the `_tnt` vertex shader uses
    /// `tintPaletteSelector.x` directly as the V coordinate, so the row centre is
    /// `(tint + 0.5) / rows`. No-op for materials without a palette.
    pub fn set_tint(&mut self, tint: u32) {
        if let Some((i, hash, rows)) = self.tint_var {
            let v = (tint.min(rows - 1) as f32 + 0.5) / rows as f32;
            if let Some(b) = self.cbuffers[i].block.as_mut() {
                b.set_f32(hash, &[v]);
            }
        }
    }

    /// Uploads dirty material buffers and binds everything for a draw.
    pub fn bind(&mut self, dev: &Device, globals: &mut Globals, samplers: &Samplers) -> Result<()> {
        for &(i, hash, var) in &self.frame_vars {
            if let Some(b) = self.cbuffers[i].block.as_mut() {
                match var {
                    FrameVar::ViewProj => b.set_f32(hash, &globals.frame.view_proj),
                    FrameVar::CableParams => b.set_f32(hash, &globals.frame.cable_params),
                };
            }
        }
        for c in &mut self.cbuffers {
            match (&c.global, &mut c.block) {
                (Some(g), _) => {
                    if let Some(b) = globals.get_mut(g) {
                        b.upload(dev)?;
                        dev.set_constant_buffer(c.stage, c.slot, b.buffer());
                    }
                }
                (None, Some(b)) => {
                    b.upload(dev)?;
                    dev.set_constant_buffer(c.stage, c.slot, b.buffer());
                }
                _ => {}
            }
        }
        for t in &self.textures {
            dev.set_shader_resource(t.stage, t.slot, &t.srv);
        }
        for (stage, slot, name) in &self.samplers {
            dev.set_sampler(*stage, *slot, samplers.for_binding(name));
        }
        Ok(())
    }

    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({
            "shader": self.shader_name,
            "technique": self.technique,
            "vs": self.vs.stage.name,
            "ps": self.ps.stage.name,
            "cbuffers": self.cbuffers.iter().map(|c| serde_json::json!({
                "stage": format!("{:?}", c.stage), "slot": c.slot,
                "name": c.global.clone().or_else(|| c.block.as_ref().map(|b| b.name.clone())),
                "global": c.global.is_some(),
            })).collect::<Vec<_>>(),
            "textures": self.textures.iter().map(|t| serde_json::json!({
                "stage": format!("{:?}", t.stage), "slot": t.slot, "binding": t.name, "texture": t.texture_name, "found": t.found,
            })).collect::<Vec<_>>(),
            "params_applied": self.params_applied,
            "params_unmatched": self.params_unmatched,
        })
    }
}
