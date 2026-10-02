//! `vespucci render`: a camera in the world, through the game's shaders, to a PNG.

use anyhow::Result;
use glam::Vec3;
use std::path::Path;
use vespucci_d3d11::Device;
use vespucci_game::GameFs;
use vespucci_render::world_view::{render_world, Lighting, WorldViewOptions};
use vespucci_render::{Camera, ShaderCache};
use vespucci_world::{collect, ArchetypeDb, MapSet, Mode, StreamOptions, TxdParents, YmapTree};

/// "HH:MM" or "HH" to the hour the game's time flags use.
pub fn parse_hour(time: &str) -> Result<u8> {
    let h: u8 = time
        .split(':')
        .next()
        .unwrap_or("")
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("--time wants HH:MM, got {time:?}"))?;
    anyhow::ensure!(h < 24, "--time hour out of range: {time}");
    Ok(h)
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    fs: &GameFs,
    mode: Mode,
    pos: (f32, f32, f32),
    look: (f32, f32, f32),
    fov: f32,
    size: (u32, u32),
    radius: f32,
    lod_scale: f32,
    lighting: &str,
    max_draws: usize,
    budget_mb: usize,
    mip_skip: &str,
    flip_sun: bool,
    exposure: f32,
    time: &str,
    script_maps: bool,
    pick: Option<(u32, u32)>,
    id_map: Option<&Path>,
    out: &Path,
    json: bool,
) -> Result<()> {
    let t0 = std::time::Instant::now();
    let set = MapSet::build(fs, mode)?;
    let db = ArchetypeDb::build(fs)?;
    let tree = YmapTree::build(fs, &set)?;
    let txd_parents = TxdParents::build(fs)?;
    let camera_pos = vespucci_world::Vec3::new(pos.0, pos.1, pos.2);
    let hour = parse_hour(time)?;
    let (instances, stats) = crate::timed("stream", || {
        collect(
            fs,
            &tree,
            &db,
            camera_pos,
            StreamOptions {
                radius,
                lod_scale,
                include_mlo_instances: false,
                hour: Some(hour),
                include_script_maps: script_maps,
            },
        )
    })?;
    log::info!("{stats:?}");

    let lighting = match lighting {
        "none" | "unlit" => Lighting::Unlit,
        _ => Lighting::Basic,
    };
    let skips: Vec<u8> = mip_skip
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let mip = [
        skips.first().copied().unwrap_or(0),
        skips.get(1).copied().unwrap_or(1),
        skips.get(2).copied().unwrap_or(2),
    ];
    let cam = Camera {
        position: Vec3::new(pos.0, pos.1, pos.2),
        target: Vec3::new(look.0, look.1, look.2),
        fov_deg: fov,
        aspect: size.0 as f32 / size.1 as f32,
        near: 0.1,
        far: 4000.0,
    };
    let opts = WorldViewOptions {
        width: size.0,
        height: size.1,
        lighting,
        mip_skip: mip,
        max_draws,
        budget_bytes: budget_mb * 1024 * 1024,
        sun_sign: if flip_sun { -1.0 } else { 1.0 },
        background: [0.28, 0.48, 0.9, 1.0],
        exposure,
        pick,
        id_map: id_map.is_some(),
    };

    let dev = crate::timed("create device", Device::create)?;
    let mut shaders = ShaderCache::new(fs);
    let (pixels, report) = crate::timed("render", || {
        render_world(
            &dev,
            fs,
            &mut shaders,
            &db,
            &txd_parents,
            &instances,
            &cam,
            &opts,
        )
    })?;
    crate::write_png(out, size.0, size.1, &pixels)?;
    if let (Some(path), Some(img)) = (id_map, &report.id_image) {
        crate::write_png(path, size.0, size.1, img)?;
    }
    let rss = crate::files::peak_rss().unwrap_or(0);
    if json {
        println!(
            "{{\"instances\":{},\"culled\":{},\"models\":{},\"models_failed\":{},\"draws\":{},\"geometry_mb\":{:.1},\"texture_mb\":{:.1},\"budget_hit\":{},\"textures_bound\":{},\"textures_missing\":{},\"textures_engine\":{},\"peak_rss_mb\":{},\"seconds\":{:.1}}}",
            report.instances_in, report.culled, report.models_loaded, report.models_failed, report.draws,
            report.geometry_bytes as f64 / 1048576.0, report.texture_bytes as f64 / 1048576.0, report.budget_hit,
            report.textures_bound, report.textures_missing, report.textures_engine, rss, t0.elapsed().as_secs_f64()
        );
    } else {
        println!(
            "{} instances ({} culled), {} models loaded ({} failed), {} draws, {:.1} MB geometry + {:.1} MB textures{}, textures bound {} / missing {} / engine {}, peak RSS {} MB, {:.1} s total; wrote {}",
            report.instances_in, report.culled, report.models_loaded, report.models_failed, report.draws,
            report.geometry_bytes as f64 / 1048576.0, report.texture_bytes as f64 / 1048576.0,
            if report.budget_hit { " (budget hit)" } else { "" },
            report.textures_bound, report.textures_missing, report.textures_engine, rss, t0.elapsed().as_secs_f64(), out.display()
        );
    }
    Ok(())
}
