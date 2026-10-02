//! A streamed set of entity instances through the game's shaders to an image.

use crate::camera::Camera;
use crate::frustum::Frustum;
use crate::globals_preview;
use crate::layout::{LayoutCache, DUMMY_BYTES, DUMMY_SLOT};
use crate::material::{Globals, Material};
use crate::shader_cache::ShaderCache;
use crate::texture::TextureCache;
use anyhow::{Context, Result};
use glam::{Mat4, Quat, Vec3};
use rage_formats::{parse_drawables, parse_ytd, Drawable, DrawableKind, LodLevel, YtdTexture};
use std::collections::HashMap;
use std::rc::Rc;
use vespucci_d3d11::ffi::*;
use vespucci_d3d11::{ComPtr, Device, Stage};
use vespucci_game::GameFs;
use vespucci_world::{ArchetypeDb, Instance, TxdParents};

/// Pixel shader that writes a draw id (picking pass); pairs with any game vertex shader.
const ID_PS: &[u8] = include_bytes!("../../../shaders/id/id.ps.dxbc");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lighting {
    Unlit,
    Basic,
}

pub struct WorldViewOptions {
    pub width: u32,
    pub height: u32,
    pub lighting: Lighting,
    /// Mip levels dropped for models nearer than 100 m, nearer than 500 m, and beyond.
    pub mip_skip: [u8; 3],
    pub max_draws: usize,
    /// Stop loading new models past this many bytes of geometry + textures.
    pub budget_bytes: usize,
    pub sun_sign: f32,
    /// Linear sky colour behind everything (there is no sky dome yet).
    pub background: [f32; 4],
    pub exposure: f32,
    /// Report the model and material drawn at this pixel (runs the picking pass).
    pub pick: Option<(u32, u32)>,
    /// Produce `WorldReport::id_image`, one colour per draw (runs the picking pass).
    pub id_map: bool,
}

#[derive(Debug, Default, Clone)]
pub struct WorldReport {
    pub instances_in: usize,
    pub culled: usize,
    pub models_loaded: usize,
    pub models_failed: usize,
    pub draws: usize,
    pub geometry_bytes: usize,
    pub texture_bytes: usize,
    pub budget_hit: bool,
    pub textures_missing: usize,
    pub textures_bound: usize,
    /// Engine-supplied inputs (reflection, fog rays) given a flat stand-in.
    pub textures_engine: usize,
    /// RGBA8, one colour per draw, black where nothing was drawn (`WorldViewOptions::id_map`).
    pub id_image: Option<Vec<u8>>,
}

struct GpuGeo {
    material: usize,
    vb: ComPtr<ID3D11Buffer>,
    stride: u32,
    ib: ComPtr<ID3D11Buffer>,
    index_format: DXGI_FORMAT,
    index_count: u32,
    layout: ComPtr<ID3D11InputLayout>,
}

struct GpuLod {
    geos: Vec<GpuGeo>,
}

struct GpuModel {
    drawable: Drawable,
    materials: Vec<Option<Material>>,
    lods: HashMap<u8, Rc<GpuLod>>,
    bytes: usize,
}

struct Ctx<'a> {
    dev: &'a Device,
    fs: &'a GameFs,
    shaders: &'a mut ShaderCache,
    db: &'a ArchetypeDb,
    txd_parents: &'a TxdParents,
    globals: Globals,
    textures: TextureCache,
    layouts: LayoutCache,
    ytds: HashMap<u32, Rc<Vec<YtdTexture>>>,
    models: HashMap<(u32, u32), Option<Rc<std::cell::RefCell<GpuModel>>>>,
    lighting: Lighting,
    report: WorldReport,
}

/// Dictionaries every model may draw on, after its own: the shared detail
/// normal maps (`env_bark`, ...) and the vehicle sheet.
const GLOBAL_TXDS: [&str; 2] = ["mapdetail", "vehshare"];

/// Technique names to try, best first, for a render bucket. Cutout (3) wants
/// the alpha-tested variants; everything else the plain forward ones.
fn technique_candidates(lighting: Lighting, bucket: u8) -> Vec<&'static str> {
    let mut v = Vec::new();
    match lighting {
        Lighting::Unlit => v.extend(["unlit_draw", "draw"]),
        Lighting::Basic => {
            if bucket == 3 {
                v.extend([
                    "lightweightHighQuality0CutOut_draw",
                    "lightweight0CutOut_draw",
                ]);
            }
            v.extend([
                "lightweightHighQuality0_draw",
                "lightweight0_draw",
                "draw",
                "unlit_draw",
            ]);
        }
    }
    v
}

impl Ctx<'_> {
    fn ytd(&mut self, hash: u32) -> Option<Rc<Vec<YtdTexture>>> {
        if let Some(t) = self.ytds.get(&hash) {
            return Some(t.clone());
        }
        let loc = self.fs.by_hash("ytd", hash)?;
        let parsed = match self.fs.read(loc).and_then(|d| parse_ytd(&d)) {
            Ok(t) => Rc::new(t),
            Err(e) => {
                log::debug!("ytd {}: {e:#}", loc.name);
                Rc::new(Vec::new())
            }
        };
        self.ytds.insert(hash, parsed.clone());
        Some(parsed)
    }

    /// Dictionaries to search for a model's textures, nearest first: the
    /// archetype's, its parents, then a dictionary named after the model.
    fn texture_sources(&mut self, archetype: u32, model_hash: u32) -> Vec<Rc<Vec<YtdTexture>>> {
        let mut hashes = Vec::new();
        if let Some(a) = self.db.get(archetype) {
            if a.texture_dict_hash != 0 {
                hashes.extend(self.txd_parents.chain(a.texture_dict_hash));
            }
        }
        if !hashes.contains(&model_hash) {
            hashes.push(model_hash);
        }
        hashes.extend(GLOBAL_TXDS.iter().map(|n| vespucci_game::joaat(n)));
        hashes.into_iter().filter_map(|h| self.ytd(h)).collect()
    }

    fn model(&mut self, archetype: u32, mip_skip: u8) -> Option<Rc<std::cell::RefCell<GpuModel>>> {
        let a = self.db.get(archetype)?;
        let (ext, hash) = a.model_file();
        let key = (vespucci_game::joaat(ext), hash);
        if let Some(m) = self.models.get(&key) {
            return m.clone();
        }
        let built = self.load_model(archetype, ext, hash, a.asset_name_hash, mip_skip);
        let entry = match built {
            Ok(m) => {
                self.report.models_loaded += 1;
                Some(Rc::new(std::cell::RefCell::new(m)))
            }
            Err(e) => {
                self.report.models_failed += 1;
                log::debug!("model {ext} {hash:#010x}: {e:#}");
                None
            }
        };
        self.models.insert(key, entry.clone());
        entry
    }

    fn load_model(
        &mut self,
        archetype: u32,
        ext: &str,
        hash: u32,
        entry_hash: u32,
        mip_skip: u8,
    ) -> Result<GpuModel> {
        let loc = self
            .fs
            .by_hash(ext, hash)
            .with_context(|| format!("no .{ext} with hash {hash:#010x}"))?;
        let data = self.fs.read(loc)?;
        let kind = DrawableKind::from_extension(ext).unwrap();
        let mut entries = parse_drawables(&data, kind)?;
        anyhow::ensure!(!entries.is_empty(), "no drawables");
        let idx = if ext == "ydd" {
            match entries.iter().position(|d| d.hash == entry_hash) {
                Some(i) => i,
                None => {
                    log::debug!("ydd {}: entry {entry_hash:#010x} not among its {} entries ({:x?}); drawing entry 0", loc.name, entries.len(), entries.iter().map(|d| d.hash).take(6).collect::<Vec<_>>());
                    0
                }
            }
        } else {
            0
        };
        let drawable = entries.swap_remove(idx.min(entries.len() - 1)).drawable;

        let sources = self.texture_sources(archetype, hash);
        let mut materials = Vec::new();
        let lighting = self.lighting;
        if let Some(sg) = &drawable.shader_group {
            let embedded = &sg.textures;
            for fx in &sg.shaders {
                let find = |h: u32| -> Option<YtdTexture> {
                    embedded
                        .iter()
                        .find(|t| t.name_hash == h)
                        .cloned()
                        .or_else(|| {
                            sources
                                .iter()
                                .find_map(|s| s.iter().find(|t| t.name_hash == h).cloned())
                        })
                };
                let built = (|| -> Result<Material> {
                    let program = self.shaders.get(self.fs, fx.name_hash)?;
                    let mut program = program.borrow_mut();
                    // Debug: VESPUCCI_SKIP_SHADER=a,b leaves out materials using these shaders.
                    if let Ok(list) = std::env::var("VESPUCCI_SKIP_SHADER") {
                        if list
                            .split(',')
                            .any(|s| s.trim().eq_ignore_ascii_case(&program.name))
                        {
                            anyhow::bail!("skipped by VESPUCCI_SKIP_SHADER");
                        }
                    }
                    let techniques = technique_candidates(lighting, fx.render_bucket);
                    let technique = program
                        .pick_technique(&techniques)
                        .with_context(|| format!("{}: none of {:?}", program.name, techniques))?
                        .to_string();
                    let (vs, ps) = program.technique_stages(self.dev, &technique)?;
                    let name = program.name.clone();
                    Material::build(
                        self.dev,
                        &program.fxc,
                        &name,
                        &technique,
                        vs,
                        ps,
                        fx,
                        &mut self.globals,
                        &mut self.textures,
                        find,
                        mip_skip,
                    )
                })();
                match built {
                    Ok(m) => {
                        for t in &m.textures {
                            if t.found {
                                self.report.textures_bound += 1;
                            } else if t.engine {
                                self.report.textures_engine += 1;
                            } else {
                                self.report.textures_missing += 1;
                                log::debug!("missing texture: shader {} binding {} texture {:?} archetype {:#010x} model {:#010x}", m.shader_name, t.name, t.texture_name, archetype, hash);
                            }
                        }
                        materials.push(Some(m));
                    }
                    Err(e) => {
                        log::debug!("material {:#010x}: {e:#}", fx.name_hash);
                        materials.push(None);
                    }
                }
            }
        }
        Ok(GpuModel {
            drawable,
            materials,
            lods: HashMap::new(),
            bytes: 0,
        })
    }

    fn lod(&mut self, model: &Rc<std::cell::RefCell<GpuModel>>, level: u8) -> Result<Rc<GpuLod>> {
        if let Some(l) = model.borrow().lods.get(&level) {
            return Ok(l.clone());
        }
        let mut m = model.borrow_mut();
        let lod_level = [
            LodLevel::High,
            LodLevel::Medium,
            LodLevel::Low,
            LodLevel::VeryLow,
        ][level as usize];
        let lod = m
            .drawable
            .lods
            .iter()
            .find(|l| l.level == lod_level)
            .or_else(|| m.drawable.lods.first())
            .context("no lods")?;
        let mut geos = Vec::new();
        let mut bytes = 0;
        for model_part in &lod.models {
            // Bit 0 of a model's render mask is the visible pass; shadow proxies clear it.
            if model_part.render_mask_flags & 1 == 0 {
                continue;
            }
            for geo in &model_part.geometries {
                let (Some(vb), Some(ib)) = (&geo.vertex_buffer, &geo.index_buffer) else {
                    continue;
                };
                let Some(decl) = &vb.declaration else {
                    continue;
                };
                let Some(Some(material)) = m.materials.get(geo.shader_id as usize) else {
                    continue;
                };
                let inputs = material
                    .vs
                    .stage
                    .inputs
                    .as_ref()
                    .context("vs without input signature")?;
                let (layout, _) =
                    self.layouts
                        .get(self.dev, decl, inputs, &material.vs.stage.dxbc)?;
                let vbuf = self.dev.create_buffer(
                    &vb.data,
                    D3D11_BIND_VERTEX_BUFFER,
                    D3D11_USAGE_IMMUTABLE,
                )?;
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
                let ibuf = self.dev.create_buffer(
                    &ibytes,
                    D3D11_BIND_INDEX_BUFFER,
                    D3D11_USAGE_IMMUTABLE,
                )?;
                bytes += vb.data.len() + ibytes.len();
                geos.push(GpuGeo {
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
        self.report.geometry_bytes += bytes;
        m.bytes += bytes;
        let l = Rc::new(GpuLod { geos });
        m.lods.insert(level, l.clone());
        Ok(l)
    }
}

/// Drawable LOD for a distance: the first level whose threshold the distance is under.
/// `VAR=x,y` from the environment, as a pixel position.
fn parse_pixel(var: &str) -> Option<(usize, usize)> {
    let p = std::env::var(var).ok()?;
    let (a, b) = p.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn pick_lod(d: &Drawable, distance: f32) -> Option<u8> {
    for (i, &threshold) in d.lod_distances.iter().enumerate() {
        if distance < threshold
            && d.lods.iter().any(|l| {
                l.level
                    == [
                        LodLevel::High,
                        LodLevel::Medium,
                        LodLevel::Low,
                        LodLevel::VeryLow,
                    ][i]
            })
        {
            return Some(i as u8);
        }
    }
    // Beyond every threshold: the coarsest level that exists, if the entity itself is still in range.
    d.lods
        .iter()
        .map(|l| match l.level {
            LodLevel::High => 0,
            LodLevel::Medium => 1,
            LodLevel::Low => 2,
            LodLevel::VeryLow => 3,
        })
        .max()
}

struct Draw {
    model: Rc<std::cell::RefCell<GpuModel>>,
    lod: Rc<GpuLod>,
    /// Model file name, for the pick diagnostic.
    name: String,
    world: Mat4,
    distance: f32,
}

/// Which pass a bucket is drawn in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pass {
    Opaque,
    Decal,
    Alpha,
}

fn pass_of(bucket: u8) -> Pass {
    match bucket {
        1 => Pass::Alpha,
        2 => Pass::Decal,
        _ => Pass::Opaque,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_world(
    dev: &Device,
    fs: &GameFs,
    shaders: &mut ShaderCache,
    db: &ArchetypeDb,
    txd_parents: &TxdParents,
    instances: &[Instance],
    cam: &Camera,
    opts: &WorldViewOptions,
) -> Result<(Vec<u8>, WorldReport)> {
    // Linear HDR colour, as the game's shaders emit it; tonemapped on read-back.
    let rt = dev.create_render_target(opts.width, opts.height, DXGI_FORMAT_R16G16B16A16_FLOAT)?;
    let depth = dev.create_depth_target(opts.width, opts.height)?;
    let rasterizer = dev.create_rasterizer(D3D11_CULL_NONE, false, false)?;
    let sampler = dev.create_sampler(D3D11_FILTER_ANISOTROPIC, D3D11_TEXTURE_ADDRESS_WRAP, 16)?;
    let comparison = dev.create_comparison_sampler()?;
    // Reversed-Z: nearer is greater.
    let depth_write = dev.create_depth_state(true, D3D11_COMPARISON_GREATER)?;
    let depth_test_eq = dev.create_depth_state(false, D3D11_COMPARISON_GREATER_EQUAL)?;
    let depth_test = dev.create_depth_state(false, D3D11_COMPARISON_GREATER)?;
    let blend_off = dev.create_blend_state(false)?;
    let blend_alpha = dev.create_blend_state(true)?;
    let mut ctx = Ctx {
        dev,
        fs,
        shaders,
        db,
        txd_parents,
        globals: Globals::new(),
        textures: TextureCache::new(dev)?,
        layouts: LayoutCache::new(),
        ytds: HashMap::new(),
        models: HashMap::new(),
        lighting: opts.lighting,
        report: WorldReport {
            instances_in: instances.len(),
            ..Default::default()
        },
    };

    let view = cam.view();
    let proj = cam.proj_reversed();
    let frustum = Frustum::from_clip(proj * view);

    // Debug: `VESPUCCI_SKIP=a,b` leaves out models whose file name contains any of these.
    let skip: Vec<String> = std::env::var("VESPUCCI_SKIP")
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_lowercase())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // Picking: a second pass writes draw ids, for `pick` and `id_map`.
    let want_ids = opts.pick.is_some() || opts.id_map;

    // Decide what to draw: frustum-cull by the archetype's box, load models within budget.
    let mut draws: Vec<Draw> = Vec::new();
    for inst in instances {
        let Some(a) = db.get(inst.archetype) else {
            continue;
        };
        if !skip.is_empty() {
            let (ext, hash) = a.model_file();
            if fs
                .by_hash(ext, hash)
                .is_some_and(|l| skip.iter().any(|s| l.name.to_lowercase().contains(s)))
            {
                continue;
            }
        }
        let (c, r) = a.bounding_sphere();
        let scale = inst.scale_xy.max(inst.scale_z);
        let world = Mat4::from_scale_rotation_translation(
            Vec3::new(inst.scale_xy, inst.scale_xy, inst.scale_z),
            Quat::from_xyzw(
                inst.orientation[0],
                inst.orientation[1],
                inst.orientation[2],
                inst.orientation[3],
            )
            .normalize(),
            Vec3::new(inst.position.x, inst.position.y, inst.position.z),
        );
        let centre = world.transform_point3(Vec3::new(c.x, c.y, c.z));
        if !frustum.contains_sphere(centre, r * scale) {
            ctx.report.culled += 1;
            continue;
        }
        if draws.len() >= opts.max_draws {
            break;
        }
        let total = ctx.report.geometry_bytes + ctx.textures.uploaded_bytes;
        if total > opts.budget_bytes {
            ctx.report.budget_hit = true;
            break;
        }
        let mip_skip = if inst.distance < 100.0 {
            opts.mip_skip[0]
        } else if inst.distance < 500.0 {
            opts.mip_skip[1]
        } else {
            opts.mip_skip[2]
        };
        let Some(model) = ctx.model(inst.archetype, mip_skip) else {
            continue;
        };
        let level = { pick_lod(&model.borrow().drawable, inst.distance) };
        let Some(level) = level else { continue };
        let lod = match ctx.lod(&model, level) {
            Ok(l) => l,
            Err(e) => {
                log::debug!("lod: {e:#}");
                continue;
            }
        };
        if log::log_enabled!(log::Level::Trace) {
            let (ext, hash) = a.model_file();
            let name = fs
                .by_hash(ext, hash)
                .map(|l| l.name.clone())
                .unwrap_or_default();
            let size = Vec3::new(
                a.bb_max.x - a.bb_min.x,
                a.bb_max.y - a.bb_min.y,
                a.bb_max.z - a.bb_min.z,
            );
            log::trace!("draw {name} d={:.0} lod={:?} lodDist={:.0} size={:.0}x{:.0}x{:.0} level={level} ymap={:#010x} eflags={:#010x} aflags={:#010x} time={:?}", inst.distance, inst.lod_level, inst.lod_dist, size.x, size.y, size.z, inst.ymap, inst.flags, a.flags, a.time_flags.map(|t| format!("{t:#010x}")));
        }
        draws.push(Draw {
            model,
            lod,
            name: if want_ids {
                let (ext, hash) = a.model_file();
                fs.by_hash(ext, hash)
                    .map(|l| l.name.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            },
            world,
            distance: inst.distance,
        });
    }
    ctx.report.texture_bytes = ctx.textures.uploaded_bytes;

    // One entry per geometry, keyed for the pass order: opaque grouped by model
    // (fewer pipeline changes), then decals, then alpha far-to-near.
    let mut items: Vec<(Pass, u64, usize, usize)> = Vec::new();
    for (di, d) in draws.iter().enumerate() {
        let model = d.model.borrow();
        for (gi, g) in d.lod.geos.iter().enumerate() {
            let Some(material) = model.materials[g.material].as_ref() else {
                continue;
            };
            let pass = pass_of(material.bucket);
            let key = match pass {
                Pass::Alpha => (u32::MAX - d.distance.to_bits()) as u64,
                _ => Rc::as_ptr(&d.model) as usize as u64,
            };
            items.push((pass, key, di, gi));
        }
    }
    items.sort_unstable();

    globals_preview::apply(
        &mut ctx.globals,
        cam,
        opts.width,
        opts.height,
        opts.sun_sign,
        false,
    );
    globals_preview::set_frame(&mut ctx.globals, cam, &view, &proj, opts.height, false);
    ctx.globals.upload_all(dev)?;
    let dummy = dev.create_buffer(
        &[0u8; DUMMY_BYTES],
        D3D11_BIND_VERTEX_BUFFER,
        D3D11_USAGE_IMMUTABLE,
    )?;
    dev.clear(&rt, opts.background);
    dev.clear_depth_to(&depth, 0.0);
    dev.bind_targets(&rt, Some(&depth));
    dev.set_rasterizer(&rasterizer);
    dev.set_vertex_buffer(DUMMY_SLOT, &dummy, 0);

    let mut current_pass = None;
    let mut last_draw = usize::MAX;
    for &(pass, _, di, gi) in &items {
        if current_pass != Some(pass) {
            current_pass = Some(pass);
            match pass {
                Pass::Opaque => {
                    dev.set_depth_state(&depth_write);
                    dev.set_blend_state(&blend_off);
                }
                Pass::Decal => {
                    dev.set_depth_state(&depth_test_eq);
                    dev.set_blend_state(&blend_alpha);
                }
                Pass::Alpha => {
                    dev.set_depth_state(&depth_test);
                    dev.set_blend_state(&blend_alpha);
                }
            }
        }
        let d = &draws[di];
        if last_draw != di {
            globals_preview::set_world(&mut ctx.globals, d.world, &view, &proj, false);
            last_draw = di;
        }
        let mut model = d.model.borrow_mut();
        let g = &d.lod.geos[gi];
        let Some(material) = model.materials[g.material].as_mut() else {
            continue;
        };
        dev.set_pipeline(&g.layout, &material.vs.shader, &material.ps.shader);
        material.bind(dev, &mut ctx.globals, &sampler, &comparison)?;
        dev.set_vertex_buffer(0, &g.vb, g.stride);
        dev.set_index_buffer(&g.ib, g.index_format);
        dev.draw_indexed(g.index_count);
        ctx.report.draws += 1;
    }
    // Picking pass: the same draws again, with a pixel shader that writes the draw id.
    if want_ids {
        let rt_id = dev.create_render_target(opts.width, opts.height, DXGI_FORMAT_R32_UINT)?;
        let id_ps = dev.create_pixel_shader(ID_PS)?;
        let id_cb =
            dev.create_buffer(&[0u8; 16], D3D11_BIND_CONSTANT_BUFFER, D3D11_USAGE_DYNAMIC)?;
        dev.clear(&rt_id, [0.0; 4]);
        dev.bind_targets(&rt_id, Some(&depth));
        dev.set_blend_state(&blend_off);
        let mut current_pass = None;
        let mut last_draw = usize::MAX;
        for &(pass, _, di, gi) in &items {
            if current_pass != Some(pass) {
                current_pass = Some(pass);
                // Alpha draws wrote no depth; everything else must land on what it wrote.
                dev.set_depth_state(match pass {
                    Pass::Alpha => &depth_test,
                    _ => &depth_test_eq,
                });
            }
            let d = &draws[di];
            if last_draw != di {
                globals_preview::set_world(&mut ctx.globals, d.world, &view, &proj, false);
                last_draw = di;
            }
            let mut model = d.model.borrow_mut();
            let g = &d.lod.geos[gi];
            let Some(material) = model.materials[g.material].as_mut() else {
                continue;
            };
            dev.set_pipeline(&g.layout, &material.vs.shader, &id_ps);
            material.bind(dev, &mut ctx.globals, &sampler, &comparison)?;
            let id = ((di as u32 + 1) << 8) | (gi as u32).min(255);
            let mut bytes = [0u8; 16];
            bytes[..4].copy_from_slice(&id.to_le_bytes());
            dev.update_buffer(&id_cb, &bytes)?;
            dev.set_constant_buffer(Stage::Pixel, 0, &id_cb);
            dev.set_vertex_buffer(0, &g.vb, g.stride);
            dev.set_index_buffer(&g.ib, g.index_format);
            dev.draw_indexed(g.index_count);
        }
        let raw = dev.read_back(&rt_id.texture, 4)?;
        let ids: Vec<u32> = raw
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if let Some((px, py)) = opts.pick {
            let id = ids
                .get(py as usize * opts.width as usize + px as usize)
                .copied()
                .unwrap_or(0);
            if id == 0 {
                log::info!("pick ({px},{py}): nothing drawn there");
            } else {
                let (di, gi) = ((id >> 8) as usize - 1, (id & 255) as usize);
                let d = &draws[di];
                let model = d.model.borrow();
                let g = &d.lod.geos[gi];
                log::info!(
                    "pick ({px},{py}): {} at {:.0} m, geometry {gi}, material {}",
                    d.name,
                    d.distance,
                    g.material
                );
                if let Some(m) = model.materials[g.material].as_ref() {
                    let tex: Vec<String> = m
                        .textures
                        .iter()
                        .map(|t| {
                            format!(
                                "{}={}{}",
                                t.name,
                                t.texture_name.as_deref().unwrap_or("-"),
                                if t.found {
                                    ""
                                } else if t.engine {
                                    "(engine)"
                                } else {
                                    "(MISSING)"
                                }
                            )
                        })
                        .collect();
                    log::info!(
                        "  {} / {} bucket={} unmatched={:?}",
                        m.shader_name,
                        m.technique,
                        m.bucket,
                        m.params_unmatched
                    );
                    log::info!("  {}", tex.join(" "));
                }
            }
        }
        if opts.id_map {
            let mut rgba = Vec::with_capacity(ids.len() * 4);
            for &id in &ids {
                if id == 0 {
                    rgba.extend_from_slice(&[0, 0, 0, 255]);
                } else {
                    let h = (id >> 8).wrapping_mul(2_654_435_761);
                    rgba.extend_from_slice(&[
                        (h >> 16) as u8 | 0x30,
                        (h >> 8) as u8 | 0x30,
                        h as u8 | 0x30,
                        255,
                    ]);
                }
            }
            ctx.report.id_image = Some(rgba);
        }
    }
    let hdr = dev.read_back(&rt.texture, 8)?;
    // Debug: VESPUCCI_PROBE=x,y prints the linear HDR value of one pixel.
    if let Some((x, y)) = parse_pixel("VESPUCCI_PROBE") {
        {
            let at = (y * opts.width as usize + x) * 8;
            if at + 8 <= hdr.len() {
                let v: Vec<f32> = (0..4)
                    .map(|i| {
                        crate::tonemap::half_to_f32(u16::from_le_bytes([
                            hdr[at + i * 2],
                            hdr[at + i * 2 + 1],
                        ]))
                    })
                    .collect();
                log::info!("probe ({x},{y}) hdr = {v:?}");
            }
        }
    }
    let (pixels, bad) = crate::tonemap::rgba16f_to_rgba8(&hdr, opts.exposure);
    if bad > 0 {
        log::warn!("{bad} pixels were NaN or infinite (shown magenta)");
    }
    Ok((pixels, ctx.report))
}
