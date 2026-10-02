//! Binding a drawable's material (`ShaderFx`) to a technique of its `.fxc`:
//! constant buffers from reflection, defaults from the `.fxc`, values from
//! the material, textures by name.

use crate::cbuffer::CBufferBlock;
use crate::shader_cache::{PixelStage, VertexStage};
use crate::texture::TextureCache;
use anyhow::Result;
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
}

impl Globals {
    pub fn new() -> Globals {
        Globals { blocks: HashMap::new() }
    }

    /// The block for a global cbuffer, created from this shader's reflection
    /// the first time it is seen, with the `.fxc` defaults written in.
    pub fn block(&mut self, dev: &Device, cb: &vespucci_fxc::dxbc::RdefCBuffer, fxc: &FxcFile) -> Result<&mut CBufferBlock> {
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
    for v in fxc.variables.iter().filter(|v| v.cbuffer_hash == hash && !v.values.is_empty()) {
        let floats = v.default_floats();
        if !block.set_f32(joaat(&v.name), &floats) {
            block.set_f32(joaat(&v.name.to_lowercase()), &floats);
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
    /// (stage, slot, binding name); names containing "shadow" get the comparison sampler.
    pub samplers: Vec<(Stage, u32, String)>,
    pub params_applied: usize,
    pub params_unmatched: Vec<String>,
    /// The drawable's render bucket: 0 opaque, 1 alpha, 2 decal, 3 cutout.
    pub bucket: u8,
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

        for (stage, rdef) in [(Stage::Vertex, &vs.stage.rdef), (Stage::Pixel, &ps.stage.rdef)] {
            for b in rdef.bindings.iter().filter(|b| b.kind == BindKind::CBuffer) {
                let Some(cb) = rdef.cbuffers.iter().find(|c| c.name == b.name) else { continue };
                if is_global(&cb.name) {
                    globals.block(dev, cb, fxc)?;
                    cbuffers.push(BoundCBuffer { stage, slot: b.bind_point, global: Some(cb.name.to_lowercase()), block: None });
                } else {
                    let mut block = CBufferBlock::new(dev, cb)?;
                    apply_fxc_defaults(&mut block, fxc, &cb.name);
                    for p in &fx.parameters {
                        if let ShaderParameterValue::Vectors(v) = &p.value {
                            let floats: Vec<f32> = v.iter().flat_map(|x| [x.x, x.y, x.z, x.w]).collect();
                            if block.set_f32(p.name_hash, &floats) {
                                params_applied += 1;
                                log::trace!("{shader_name}: {}:{} = {:?}", block.name, block.name_of(p.name_hash).unwrap_or("?"), &floats[..floats.len().min(8)]);
                            }
                        }
                    }
                    cbuffers.push(BoundCBuffer { stage, slot: b.bind_point, global: None, block: Some(block) });
                }
            }
        }
        // Debug: VESPUCCI_SET_VAR=cbuffer:var=a,b,c,d;... overrides material variables everywhere.
        if let Ok(spec) = std::env::var("VESPUCCI_SET_VAR") {
            for item in spec.split(';') {
                if let Some((target, values)) = item.split_once('=') {
                    if let Some((cb, var)) = target.split_once(':') {
                        let v: Vec<f32> = values.split(',').filter_map(|x| x.trim().parse().ok()).collect();
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
                let matched = cbuffers.iter().any(|c| c.block.as_ref().is_some_and(|b| b.has(p.name_hash)));
                if !matched {
                    params_unmatched.push(format!("{:#010x}", p.name_hash));
                }
            }
        }

        let mut bound_textures = Vec::new();
        let mut samplers = Vec::new();
        for (stage, rdef) in [(Stage::Vertex, &vs.stage.rdef), (Stage::Pixel, &ps.stage.rdef)] {
            for b in &rdef.bindings {
                match b.kind {
                    BindKind::Sampler => samplers.push((stage, b.bind_point, b.name.clone())),
                    BindKind::Texture => {
                        // The material parameter of the same name says which texture.
                        let (h1, h2) = (joaat(&b.name), joaat(&b.name.to_lowercase()));
                        let param = fx.parameters.iter().find(|p| p.name_hash == h1 || p.name_hash == h2);
                        let mut texture_name = None;
                        let is_shadow = b.name.to_lowercase().contains("shadow");
                        let engine = param.is_none() && !is_shadow;
                        let mut srv = if is_shadow { textures.unshadowed.clone_ptr() } else if engine { textures.engine.clone_ptr() } else { textures.missing.clone_ptr() };
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
                            }
                        }
                        bound_textures.push(BoundTexture { stage, slot: b.bind_point, name: b.name.clone(), texture_name, srv, found, engine });
                    }
                    _ => {}
                }
            }
        }

        Ok(Material { shader_name: shader_name.to_string(), technique: technique.to_string(), vs, ps, cbuffers, textures: bound_textures, samplers, params_applied, params_unmatched, bucket: fx.render_bucket })
    }

    /// Uploads dirty material buffers and binds everything for a draw.
    pub fn bind(&mut self, dev: &Device, globals: &mut Globals, sampler: &ComPtr<ID3D11SamplerState>, comparison: &ComPtr<ID3D11SamplerState>) -> Result<()> {
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
            let s = if name.to_lowercase().contains("shadow") { comparison } else { sampler };
            dev.set_sampler(*stage, *slot, s);
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
