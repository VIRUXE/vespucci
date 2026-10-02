//! `vespucci render-model`: one drawable through the game's shaders to a PNG.

use anyhow::{bail, Context, Result};
use rage_formats::{parse_drawables, parse_ytd, DrawableKind, LodLevel, YtdTexture};
use std::path::Path;
use vespucci_d3d11::Device;
use vespucci_game::GameFs;
use vespucci_render::{render_drawable, ModelViewOptions, ShaderCache};
use vespucci_world::ArchetypeDb;

#[allow(clippy::too_many_arguments)]
pub fn run(fs: &GameFs, model: &str, entry: Option<&str>, ytds: &[String], lod: &str, techniques: &[String], size: (u32, u32), yaw: f32, pitch: f32, transpose: bool, flip_sun: bool, cull: bool, wireframe: bool, mip_skip: u8, out: &Path, dump_binding: Option<&Path>) -> Result<()> {
    // Locate the model: a game path, or a name resolved like the game does.
    let (loc, kind) = if let Some(l) = fs.get(model) {
        let kind = DrawableKind::from_extension(&l.ext).context("not a ydr/ydd/yft")?;
        (l, kind)
    } else {
        let name = model.rsplit('/').next().unwrap_or(model);
        let (stem, ext_hint) = match name.rsplit_once('.') {
            Some((s, e)) if matches!(e, "ydr" | "ydd" | "yft") => (s, Some(e)),
            _ => (name, None),
        };
        let mut found = None;
        for ext in ext_hint.map(|e| vec![e]).unwrap_or_else(|| vec!["ydr", "yft", "ydd"]) {
            if let Some(l) = fs.by_name(ext, stem) {
                found = Some((l, DrawableKind::from_extension(ext).unwrap()));
                break;
            }
        }
        found.with_context(|| format!("no model named {model}"))?
    };
    log::info!("model: {}", loc.full_path(fs));
    let data = fs.read(loc)?;
    let drawables = parse_drawables(&data, kind)?;
    let drawable = match entry {
        Some(e) => {
            let h = u32::from_str_radix(e.trim_start_matches("0x"), 16).unwrap_or_else(|_| vespucci_game::joaat(&e.to_lowercase()));
            &drawables.iter().find(|d| d.hash == h || d.name.eq_ignore_ascii_case(e)).with_context(|| format!("no entry {e}"))?.drawable
        }
        None => &drawables.first().context("file holds no drawables")?.drawable,
    };
    // Vertex colour statistics of the first geometry: the unlit shader multiplies by it.
    if let Some(geo) = drawable.lods.first().and_then(|l| l.models.first()).and_then(|m| m.geometries.first()) {
        if let Some(vb) = &geo.vertex_buffer {
            let n = (vb.vertex_count as usize).min(2000);
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            for i in 0..n {
                if let Ok(attrs) = vb.read_vertex_attributes(i) {
                    for a in attrs {
                        if a.component.semantic == rage_formats::VertexSemantic::Colour0 {
                            let c = a.value.as_rgba8();
                            for k in 0..4 {
                                sum[k] += c[k] as u64;
                            }
                            count += 1;
                        }
                    }
                }
            }
            if count > 0 {
                log::info!("vertex colour0 mean over {count} vertices: r {} g {} b {} a {}", sum[0] / count, sum[1] / count, sum[2] / count, sum[3] / count);
            }
        }
    }
    log::info!("{} lods, {} shaders, {} embedded textures", drawable.lods.len(), drawable.shader_group.as_ref().map_or(0, |s| s.shaders.len()), drawable.shader_group.as_ref().map_or(0, |s| s.textures.len()));

    // Extra textures: explicit --ytd, then the archetype's dictionary and the same-name guess.
    let mut extra: Vec<YtdTexture> = Vec::new();
    let mut ytd_names: Vec<String> = ytds.to_vec();
    if let Some(db) = ArchetypeDb::build(fs).ok() {
        if let Some(a) = db.get(loc.stem_hash) {
            if a.texture_dict_hash != 0 {
                if let Some(t) = fs.by_hash("ytd", a.texture_dict_hash) {
                    ytd_names.push(t.full_path(fs));
                }
            }
        }
    }
    if let Some(t) = fs.by_hash("ytd", loc.stem_hash) {
        ytd_names.push(t.full_path(fs));
    }
    for name in &ytd_names {
        let l = fs.get(name).or_else(|| fs.by_name("ytd", name));
        match l.map(|l| fs.read(l).and_then(|d| parse_ytd(&d))) {
            Some(Ok(mut t)) => {
                log::info!("ytd {name}: {} textures", t.len());
                extra.append(&mut t);
            }
            Some(Err(e)) => log::warn!("ytd {name}: {e:#}"),
            None => log::warn!("ytd {name}: not found"),
        }
    }

    let lod = match lod {
        "high" => LodLevel::High,
        "medium" | "med" => LodLevel::Medium,
        "low" => LodLevel::Low,
        "verylow" | "vlow" => LodLevel::VeryLow,
        other => bail!("unknown lod {other}"),
    };
    let opts = ModelViewOptions {
        width: size.0,
        height: size.1,
        lod,
        techniques: if techniques.is_empty() { vec!["unlit_draw".into(), "draw".into()] } else { techniques.to_vec() },
        yaw_deg: yaw,
        pitch_deg: pitch,
        transpose_matrices: transpose,
        cull,
        wireframe,
        mip_skip,
        background: [0.25, 0.25, 0.28, 1.0],
        sun_sign: if flip_sun { -1.0 } else { 1.0 },
    };
    let dev = crate::timed("create device", Device::create)?;
    let mut shaders = ShaderCache::new(fs);
    let (pixels, report) = crate::timed("render", || render_drawable(&dev, fs, &mut shaders, drawable, &extra, &opts))?;
    crate::write_png(out, size.0, size.1, &pixels)?;
    let fg: Vec<&[u8]> = pixels.chunks(4).filter(|p| p[0] != 64 || p[1] != 64 || p[2] != 71).collect();
    let non_bg = fg.len();
    if non_bg > 0 {
        let mean: Vec<u64> = (0..3).map(|k| fg.iter().map(|p| p[k] as u64).sum::<u64>() / non_bg as u64).collect();
        let max: Vec<u8> = (0..3).map(|k| fg.iter().map(|p| p[k]).max().unwrap()).collect();
        println!("  foreground mean rgb {:?}, max rgb {:?}", mean, max);
    }
    println!(
        "{} geometries drawn, {} skipped, dummy inputs {:?}, {:.1} MB textures, {:.1}% of pixels covered; wrote {}",
        report.geometries_drawn,
        report.geometries_skipped,
        report.dummy_inputs,
        report.textures_uploaded_bytes as f64 / 1048576.0,
        100.0 * non_bg as f64 / (size.0 * size.1) as f64,
        out.display()
    );
    for l in &report.vertex_layouts {
        println!("  vertex layout: {l}");
    }
    for m in &report.materials {
        let missing: Vec<_> = m["textures"].as_array().map(|a| a.iter().filter(|t| t["found"] == false).map(|t| format!("{}={}", t["binding"], t["texture"])).collect()).unwrap_or_default();
        println!("  {} / {}: params applied {}, unmatched {}, textures missing {:?}", m["shader"], m["technique"], m["params_applied"], m["params_unmatched"].as_array().map_or(0, |a| a.len()), missing);
    }
    if let Some(p) = dump_binding {
        std::fs::write(p, serde_json::to_string_pretty(&report.materials)?)?;
        println!("  binding report: {}", p.display());
    }
    Ok(())
}
