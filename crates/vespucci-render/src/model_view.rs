//! One drawable through the game's shaders to an image.

use crate::camera::Camera;
use crate::layout::{LayoutCache, DUMMY_BYTES, DUMMY_SLOT};
use crate::material::{Globals, Material};
use crate::shader_cache::ShaderCache;
use crate::texture::TextureCache;
use anyhow::{Context, Result};
use glam::Vec3;
use rage_formats::{Drawable, LodLevel, YtdTexture};
use std::collections::HashMap;
use vespucci_d3d11::ffi::*;
use vespucci_d3d11::{ComPtr, Device};
use vespucci_game::GameFs;

pub struct ModelViewOptions {
    pub width: u32,
    pub height: u32,
    pub lod: LodLevel,
    /// Technique names to try, most wanted first.
    pub techniques: Vec<String>,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub transpose_matrices: bool,
    pub cull: bool,
    pub wireframe: bool,
    pub mip_skip: u8,
    pub background: [f32; 4],
    /// +1 or -1: which way `gDirectionalLight` points (settled empirically).
    pub sun_sign: f32,
}

pub struct ModelViewReport {
    pub geometries_drawn: usize,
    pub geometries_skipped: usize,
    pub materials: Vec<serde_json::Value>,
    pub textures_uploaded_bytes: usize,
    pub dummy_inputs: Vec<String>,
    pub vertex_layouts: Vec<String>,
}

struct Geo {
    material: usize,
    vb: ComPtr<ID3D11Buffer>,
    stride: u32,
    ib: ComPtr<ID3D11Buffer>,
    index_format: DXGI_FORMAT,
    index_count: u32,
    layout: ComPtr<ID3D11InputLayout>,
}

/// Renders `drawable` and returns RGBA8 pixels plus what was bound.
pub fn render_drawable(
    dev: &Device,
    fs: &GameFs,
    shaders: &mut ShaderCache,
    drawable: &Drawable,
    extra_textures: &[YtdTexture],
    opts: &ModelViewOptions,
) -> Result<(Vec<u8>, ModelViewReport)> {
    let rt = dev.create_render_target(opts.width, opts.height, DXGI_FORMAT_R8G8B8A8_UNORM)?;
    let depth = dev.create_depth_target(opts.width, opts.height)?;
    let rasterizer = dev.create_rasterizer(
        if opts.cull {
            D3D11_CULL_BACK
        } else {
            D3D11_CULL_NONE
        },
        false,
        opts.wireframe,
    )?;
    let sampler = dev.create_sampler(D3D11_FILTER_ANISOTROPIC, D3D11_TEXTURE_ADDRESS_WRAP, 16)?;
    let comparison = dev.create_comparison_sampler()?;
    let mut textures = TextureCache::new(dev)?;
    let mut globals = Globals::new();
    let mut layouts = LayoutCache::new();

    // Textures: the drawable's own first, then any extra dictionaries given.
    let mut by_hash: HashMap<u32, &YtdTexture> = HashMap::new();
    for t in extra_textures {
        by_hash.entry(t.name_hash).or_insert(t);
    }
    if let Some(sg) = &drawable.shader_group {
        for t in &sg.textures {
            by_hash.insert(t.name_hash, t);
        }
    }
    let find_texture = |h: u32| by_hash.get(&h).map(|t| (*t).clone());

    // One material per ShaderFx in the group.
    let mut materials: Vec<Option<Material>> = Vec::new();
    let techniques: Vec<&str> = opts.techniques.iter().map(|s| s.as_str()).collect();
    if let Some(sg) = &drawable.shader_group {
        for fx in &sg.shaders {
            let built = (|| -> Result<Material> {
                let program = shaders
                    .get(fs, fx.name_hash)
                    .with_context(|| format!("shader {:#010x}", fx.name_hash))?;
                let mut program = program.borrow_mut();
                let technique = program
                    .pick_technique(&techniques)
                    .with_context(|| format!("{}: none of {:?}", program.name, techniques))?
                    .to_string();
                let (vs, ps) = program.technique_stages(dev, &technique)?;
                let name = program.name.clone();
                Material::build(
                    dev,
                    &program.fxc,
                    &name,
                    &technique,
                    vs,
                    ps,
                    fx,
                    &mut globals,
                    &mut textures,
                    find_texture,
                    opts.mip_skip,
                )
            })();
            match built {
                Ok(m) => materials.push(Some(m)),
                Err(e) => {
                    log::warn!("material {:#010x}: {e:#}", fx.name_hash);
                    materials.push(None);
                }
            }
        }
    }

    // Geometry of the chosen LOD.
    let mut geos = Vec::new();
    let mut skipped = 0;
    let mut dummy_inputs: Vec<String> = Vec::new();
    let mut vertex_layouts: Vec<String> = Vec::new();
    let lod = drawable
        .lods
        .iter()
        .find(|l| l.level == opts.lod)
        .or_else(|| drawable.lods.first())
        .context("drawable has no LODs")?;
    for model in &lod.models {
        if model.render_mask_flags & 1 == 0 && std::env::var("VESPUCCI_ALL_MODELS").is_err() {
            continue; // shadow-only model (render mask bit 0 clear)
        }
        for geo in &model.geometries {
            let Some(vb) = &geo.vertex_buffer else {
                skipped += 1;
                continue;
            };
            let Some(decl) = &vb.declaration else {
                skipped += 1;
                continue;
            };
            let Some(ib) = &geo.index_buffer else {
                skipped += 1;
                continue;
            };
            let Some(Some(material)) = materials.get(geo.shader_id as usize) else {
                skipped += 1;
                continue;
            };
            let inputs = material
                .vs
                .stage
                .inputs
                .as_ref()
                .context("vs without input signature")?;
            let (layout, dummies) = match layouts.get(dev, decl, inputs, &material.vs.stage.dxbc) {
                Ok(l) => l,
                Err(e) => {
                    log::warn!("input layout: {e:#}");
                    skipped += 1;
                    continue;
                }
            };
            for d in dummies {
                if !dummy_inputs.contains(&d) {
                    dummy_inputs.push(d);
                }
            }
            let desc = decl
                .components
                .iter()
                .map(|c| {
                    format!(
                        "{}:{}@{}",
                        c.semantic.as_str(),
                        c.component_type.as_str(),
                        c.offset
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            if !vertex_layouts.contains(&desc) {
                vertex_layouts.push(desc);
            }
            let vbuf =
                dev.create_buffer(&vb.data, D3D11_BIND_VERTEX_BUFFER, D3D11_USAGE_IMMUTABLE)?;
            let (ibytes, index_format) = if ib.indices.iter().all(|&i| i < 65536) {
                (
                    ib.indices
                        .iter()
                        .flat_map(|&i| (i as u16).to_le_bytes())
                        .collect::<Vec<u8>>(),
                    DXGI_FORMAT_R16_UINT,
                )
            } else {
                (
                    ib.indices
                        .iter()
                        .flat_map(|&i| i.to_le_bytes())
                        .collect::<Vec<u8>>(),
                    DXGI_FORMAT_R32_UINT,
                )
            };
            let ibuf =
                dev.create_buffer(&ibytes, D3D11_BIND_INDEX_BUFFER, D3D11_USAGE_IMMUTABLE)?;
            geos.push(Geo {
                material: geo.shader_id as usize,
                vb: vbuf,
                stride: vb.vertex_stride as u32,
                ib: ibuf,
                index_format,
                index_count: ib.indices.len() as u32,
                layout,
            });
        }
    }

    // Camera framing the drawable, and the engine globals every shader shares.
    let b = &drawable.bounds;
    let centre = Vec3::new(b.center.x, b.center.y, b.center.z);
    let radius = if b.sphere_radius > 0.0 {
        b.sphere_radius
    } else {
        1.0
    };
    let cam = Camera::orbit(
        centre,
        radius,
        opts.width as f32 / opts.height as f32,
        opts.yaw_deg,
        opts.pitch_deg,
    );
    crate::globals_preview::apply(
        &mut globals,
        &cam,
        opts.width,
        opts.height,
        opts.sun_sign,
        opts.transpose_matrices,
    );
    // Debug: VESPUCCI_SET_VAR=<cbuffer>:<var>=a,b,c,d sets a material variable in every material.
    if let Ok(spec) = std::env::var("VESPUCCI_SET_VAR") {
        if let Some((target, values)) = spec.split_once('=') {
            if let Some((cb, var)) = target.split_once(':') {
                let v: Vec<f32> = values
                    .split(',')
                    .filter_map(|x| x.trim().parse().ok())
                    .collect();
                let mut hits = 0;
                for m in materials.iter_mut().flatten() {
                    for c in &mut m.cbuffers {
                        if let Some(b) = &mut c.block {
                            if b.name == cb && b.set_by_name(var, &v) {
                                hits += 1;
                            }
                        }
                    }
                }
                log::info!(
                    "VESPUCCI_SET_VAR {cb}:{var} = {v:?} applied to {hits} material buffers"
                );
            }
        }
    }
    // Debug: VESPUCCI_ONES_VAR=<cbuffer>:<var> sets one variable to all 1.0.
    if let Ok(spec) = std::env::var("VESPUCCI_ONES_VAR") {
        let parts: Vec<&str> = spec.split(':').collect();
        if parts.len() >= 2 {
            if let Some(b) = globals.get_mut(parts[0]) {
                match parts.get(2).and_then(|i| i.parse::<usize>().ok()) {
                    Some(i) => {
                        let mut v = vec![0.0f32; i + 1];
                        if parts[1] == "globalFogParams" {
                            v[0] = 1.0e8;
                        }
                        v[i] = 1.0;
                        b.set_by_name(parts[1], &v);
                    }
                    None => {
                        b.set_by_name(parts[1], &[1.0; 64]);
                    }
                }
            }
        }
    }
    // Debug: VESPUCCI_ONES=<global cbuffer name>|material fills that buffer with 1.0.
    if let Ok(name) = std::env::var("VESPUCCI_ONES") {
        if name == "material" {
            for m in materials.iter_mut().flatten() {
                for c in &mut m.cbuffers {
                    if let Some(b) = &mut c.block {
                        b.fill_f32(1.0);
                    }
                }
            }
        } else if let Some(b) = globals.get_mut(&name) {
            b.fill_f32(1.0);
        }
    }
    globals.upload_all(dev)?;

    let dummy = dev.create_buffer(
        &[0u8; DUMMY_BYTES],
        D3D11_BIND_VERTEX_BUFFER,
        D3D11_USAGE_IMMUTABLE,
    )?;
    dev.clear(&rt, opts.background);
    dev.clear_depth(&depth);
    dev.bind_targets(&rt, Some(&depth));
    dev.set_rasterizer(&rasterizer);
    dev.set_vertex_buffer(DUMMY_SLOT, &dummy, 0);
    let mut drawn = 0;
    for g in &geos {
        let material = materials[g.material].as_mut().unwrap();
        dev.set_pipeline(&g.layout, &material.vs.shader, &material.ps.shader);
        material.bind(dev, &mut globals, &sampler, &comparison)?;
        dev.set_vertex_buffer(0, &g.vb, g.stride);
        dev.set_index_buffer(&g.ib, g.index_format);
        dev.draw_indexed(g.index_count);
        drawn += 1;
    }
    let pixels = dev.read_back(&rt.texture, 4)?;
    Ok((
        pixels,
        ModelViewReport {
            geometries_drawn: drawn,
            geometries_skipped: skipped,
            materials: materials.iter().flatten().map(|m| m.report()).collect(),
            textures_uploaded_bytes: textures.uploaded_bytes,
            dummy_inputs,
            vertex_layouts,
        },
    ))
}
