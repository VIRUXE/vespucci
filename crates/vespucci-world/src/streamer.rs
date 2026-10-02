//! Which entities a camera sees: the maps touching its range, their
//! entities, and the game's LOD rule applied by distance.

use crate::archetypes::ArchetypeDb;
use crate::entities::{parse_entities, Entity, LodLevel};
use crate::ymaps::YmapTree;
use anyhow::Result;
use rage_formats::Vec3;
use rayon::prelude::*;
use vespucci_game::GameFs;

#[derive(Debug, Clone)]
pub struct Instance {
    pub archetype: u32,
    pub position: Vec3,
    /// World rotation (x, y, z, w).
    pub orientation: [f32; 4],
    pub scale_xy: f32,
    pub scale_z: f32,
    pub distance: f32,
    pub lod_level: LodLevel,
    pub lod_dist: f32,
    pub tint: u32,
    pub ymap: u32,
    /// `CEntityDef::flags` as stored.
    pub flags: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct StreamOptions {
    /// Maps whose entity extents touch this sphere around the camera are read.
    pub radius: f32,
    /// Multiplies every entity's LOD distances (1.0 = the game's).
    pub lod_scale: f32,
    pub include_mlo_instances: bool,
    /// Hour of day (0..24) for time-dependent archetypes; `None` shows them all.
    pub hour: Option<u8>,
    /// Also read maps the game only loads on a script's request (`CMapData` flag bit 0).
    pub include_script_maps: bool,
}

#[derive(Debug, Default, Clone)]
pub struct StreamStats {
    pub ymaps_touched: usize,
    pub ymaps_failed: usize,
    pub entities_seen: usize,
    pub beyond_lod_dist: usize,
    pub hidden_by_children: usize,
    pub unknown_archetype: usize,
    pub mlo_skipped: usize,
    /// Entities whose archetype is off at the requested hour.
    pub time_hidden: usize,
    /// Maps left out because the game only loads them on a script's request.
    pub script_maps_skipped: usize,
    pub instances: usize,
}

/// The LOD rule: an entity shows when the camera is within its `lodDist`
/// and, if it has children, farther than `childLodDist` (then the children
/// show instead).
pub fn visible(e: &Entity, distance: f32, lod_scale: f32) -> (bool, &'static str) {
    if distance >= e.lod_dist * lod_scale {
        return (false, "beyond");
    }
    if e.num_children > 0 && distance < e.child_lod_dist * lod_scale {
        return (false, "children");
    }
    (true, "visible")
}

/// The time-of-day suffix of a map name (`vb_29_day` -> `day`), ignoring a trailing `_lod`.
fn time_variant(stem: &str) -> Option<(&str, &str)> {
    let base = stem.strip_suffix("_lod").unwrap_or(stem);
    for v in ["morning", "day", "evening", "night"] {
        if let Some(b) = base.strip_suffix(v) {
            if let Some(b) = b.strip_suffix('_') {
                return Some((b, v));
            }
        }
    }
    None
}

/// Which time variant a script would have loaded at this hour, given the
/// variants that exist for the map. The game's scripts swap these maps
/// (market stalls, beach crowds); the hour bands are Vespucci's guess.
fn wanted_variant(hour: u8, available: &[&str]) -> &'static str {
    let has = |v: &str| available.contains(&v);
    match hour {
        6..=11 if has("morning") => "morning",
        6..=11 => "day",
        12..=17 => "day",
        18..=20 if has("evening") => "evening",
        18..=20 => "day",
        _ => "night",
    }
}

pub fn collect(fs: &GameFs, tree: &YmapTree, db: &ArchetypeDb, camera: Vec3, opts: StreamOptions) -> Result<(Vec<Instance>, StreamStats)> {
    let maps = tree.touching(camera, opts.radius);
    // Time-of-day map variants: which suffixes exist per base name.
    let stem_of = |file: u32| -> String { fs.files[file as usize].name.trim_end_matches(".ymap").to_lowercase() };
    let mut variants: std::collections::HashMap<String, Vec<&'static str>> = std::collections::HashMap::new();
    for m in &maps {
        let stem = stem_of(m.file);
        if let Some((base, v)) = time_variant(&stem) {
            let key = format!("{base}{}", if stem.ends_with("_lod") { "_lod" } else { "" });
            let v: &'static str = ["morning", "day", "evening", "night"].into_iter().find(|x| *x == v).unwrap();
            variants.entry(key).or_default().push(v);
        }
    }
    let parsed: Vec<(u32, Result<crate::entities::YmapEntities>)> = maps.par_iter().map(|m| (m.file, fs.read(&fs.files[m.file as usize]).and_then(|d| parse_entities(&d)))).collect();
    let mut stats = StreamStats { ymaps_touched: maps.len(), ..Default::default() };
    let mut out = Vec::new();
    for (file, r) in parsed {
        let ents = match r {
            Ok(p) => {
                log::debug!("ymap {} flags={:#x} content={:#x} entities={}", fs.files[file as usize].name, p.flags, p.content_flags, p.entities.len());
                if p.flags & 1 != 0 && !opts.include_script_maps {
                    // Script-requested map: only the time-of-day variants are predictable.
                    let stem = stem_of(file);
                    let keep = match (time_variant(&stem), opts.hour) {
                        (Some((base, v)), Some(h)) => {
                            let key = format!("{base}{}", if stem.ends_with("_lod") { "_lod" } else { "" });
                            wanted_variant(h, variants.get(&key).map(|v| v.as_slice()).unwrap_or(&[])) == v
                        }
                        _ => false,
                    };
                    if !keep {
                        stats.script_maps_skipped += 1;
                        continue;
                    }
                }
                p.entities
            }
            Err(e) => {
                stats.ymaps_failed += 1;
                log::debug!("{}: {e:#}", fs.files[file as usize].name);
                continue;
            }
        };
        for e in &ents {
            stats.entities_seen += 1;
            if e.is_mlo_instance && !opts.include_mlo_instances {
                stats.mlo_skipped += 1;
                continue;
            }
            let d = dist(e.position, camera);
            match visible(e, d, opts.lod_scale) {
                (false, "beyond") => {
                    stats.beyond_lod_dist += 1;
                    continue;
                }
                (false, _) => {
                    stats.hidden_by_children += 1;
                    continue;
                }
                _ => {}
            }
            let Some(a) = db.get(e.archetype_hash) else {
                stats.unknown_archetype += 1;
                continue;
            };
            if let (Some(h), Some(t)) = (opts.hour, a.time_flags) {
                if (t >> (h % 24)) & 1 == 0 {
                    stats.time_hidden += 1;
                    continue;
                }
            }
            out.push(Instance {
                archetype: e.archetype_hash,
                position: e.position,
                orientation: e.orientation(),
                scale_xy: e.scale_xy,
                scale_z: e.scale_z,
                distance: d,
                lod_level: e.lod_level,
                lod_dist: e.lod_dist,
                tint: e.tint,
                ymap: file,
                flags: e.flags,
            });
        }
    }
    stats.instances = out.len();
    out.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    Ok((out, stats))
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    (dx * dx + dy * dy + dz * dz).sqrt()
}
