//! `index`, `probe` and `cover`: the world's data without a GPU.

use anyhow::Result;
use rage_formats::ymap::rotate;
use std::collections::BTreeMap;
use vespucci_game::GameFs;
use vespucci_world::entities::stored_rotation_to_world;
use vespucci_world::streamer::interior_world;
use vespucci_world::{
    child_lod_dist, lod_dist, parse_entities, ArchetypeDb, ArchetypeRec, LodLevel, MapSet, Mode,
    Vec3, YmapNode, YmapTree,
};

pub fn index(fs: &GameFs, mode: Mode) -> Result<()> {
    let t = std::time::Instant::now();
    let set = MapSet::build(fs, mode)?;
    let db = ArchetypeDb::build(fs)?;
    let tree = YmapTree::build(fs, &set)?;
    println!(
        "map set {mode:?}: {} packs read, {} pack archives enabled, {} base archives invalidated",
        set.packs_read, set.containers_enabled, set.containers_invalidated
    );
    println!(
        "archetypes: {} (from {} ytyps, {} failed)",
        db.by_hash.len(),
        db.ytyps_parsed,
        db.ytyps_failed
    );
    println!(
        "ymaps: {} ({} from cache files, {} from headers, {} failed)",
        tree.nodes.len(),
        tree.from_cache,
        tree.from_headers,
        tree.failed
    );
    let mlo = db.by_hash.values().filter(|a| a.is_mlo).count();
    let roots = tree.nodes.iter().filter(|n| n.parent_hash == 0).count();
    println!(
        "  {mlo} MLO archetypes; {roots} root ymaps; {} ymaps with children",
        tree.children.len()
    );
    if let Some(mb) = crate::files::peak_rss() {
        println!("  peak RSS {mb} MB, {:.1} s", t.elapsed().as_secs_f64());
    }
    Ok(())
}

/// What a camera at `pos` would stream: maps within `radius`, their entity
/// counts per LOD level, and how much model data that would touch.
pub fn probe(fs: &GameFs, mode: Mode, pos: Vec3, radius: f32, json: bool) -> Result<()> {
    let set = MapSet::build(fs, mode)?;
    let db = ArchetypeDb::build(fs)?;
    let tree = YmapTree::build(fs, &set)?;
    let maps = tree.touching(pos, radius);
    let mut per_level: BTreeMap<&str, usize> = BTreeMap::new();
    let mut total_entities = 0usize;
    let mut cargens = 0usize;
    let mut unknown_archetypes = 0usize;
    let mut model_bytes = 0u64;
    let mut models_seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for m in &maps {
        let loc = &fs.files[m.file as usize];
        let parsed = match fs.read(loc).and_then(|d| parse_entities(&d)) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("{}: {e:#}", loc.name);
                continue;
            }
        };
        let mut counts = [0usize; 7];
        for e in &parsed.entities {
            counts[e.lod_level as usize] += 1;
            *per_level.entry(level_name(e.lod_level)).or_default() += 1;
            match db.get(e.archetype_hash) {
                Some(a) => {
                    let (ext, hash) = a.model_file();
                    if models_seen.insert((ext, hash)) {
                        if let Some(f) = fs.by_hash(ext, hash) {
                            model_bytes += f.mem_size as u64;
                        }
                    }
                }
                None => unknown_archetypes += 1,
            }
        }
        total_entities += parsed.entities.len();
        cargens += parsed.car_generators.len();
        rows.push((
            loc.name.clone(),
            parsed.entities.len(),
            counts,
            parsed.car_generators.len(),
            content(m),
        ));
    }
    rows.sort_by(|a, b| b.1.cmp(&a.1));

    if json {
        println!("{{\"maps\":{},\"entities\":{},\"car_generators\":{},\"unknown_archetypes\":{},\"model_mb\":{:.1},\"per_level\":{:?}}}", maps.len(), total_entities, cargens, unknown_archetypes, model_bytes as f64 / 1048576.0, per_level);
        return Ok(());
    }
    println!(
        "{} ymaps touch a {radius} m sphere at ({}, {}, {}):",
        maps.len(),
        pos.x,
        pos.y,
        pos.z
    );
    println!(
        "{:<40} {:>6}  {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5}  {:>4}  content",
        "ymap", "ents", "HD", "LOD", "SLOD1", "SLOD2", "SLOD3", "ORPH", "SLOD4", "cars"
    );
    for (name, n, c, cars, flags) in &rows {
        println!(
            "{:<40} {:>6}  {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5}  {:>4}  {}",
            name, n, c[0], c[1], c[2], c[3], c[4], c[5], c[6], cars, flags
        );
    }
    println!("{total_entities} entities, {cargens} car generators, {unknown_archetypes} with unknown archetypes; {} distinct models, {:.1} MB of model data", models_seen.len(), model_bytes as f64 / 1048576.0);
    Ok(())
}

/// Which entities cover a point: every entity, in every map of the set, whose
/// world-space bounding box (the archetype's box through the entity's
/// transform) contains `at` grown by `margin`, with the LOD fields the
/// streaming rule reads. Answers "what should be drawn here?" without a camera.
pub fn cover(fs: &GameFs, mode: Mode, at: Vec3, margin: f32) -> Result<()> {
    let set = MapSet::build(fs, mode)?;
    let db = ArchetypeDb::build(fs)?;
    let tree = YmapTree::build(fs, &set)?;
    let inside = |min: Vec3, max: Vec3, m: f32| {
        at.x >= min.x - m
            && at.x <= max.x + m
            && at.y >= min.y - m
            && at.y <= max.y + m
            && at.z >= min.z - m
            && at.z <= max.z + m
    };
    // Only maps whose entity extents reach the point can hold such an entity.
    let maps: Vec<&YmapNode> = tree
        .nodes
        .iter()
        .filter(|n| inside(n.entities_min, n.entities_max, margin + 1.0))
        .collect();
    let model_name = |a: &ArchetypeRec| -> String {
        let (ext, h) = a.model_file();
        fs.by_hash(ext, h)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| format!("{h:#010x}.{ext}"))
    };
    let mut rows: Vec<CoverRow> = Vec::new();
    for n in &maps {
        let loc = &fs.files[n.file as usize];
        let parsed = match fs.read(loc).and_then(|d| parse_entities(&d)) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("{}: {e:#}", loc.name);
                continue;
            }
        };
        let script = parsed.flags & 1 != 0;
        for (i, e) in parsed.entities.iter().enumerate() {
            let Some(a) = db.get(e.archetype_hash) else {
                continue;
            };
            let (bmin, bmax) = world_box(a, e.scale_xy, e.scale_z, e.orientation(), e.position);
            let lod = lod_dist(e, Some(a));
            let def = e
                .is_mlo_instance
                .then(|| db.mlo(e.archetype_hash))
                .flatten();
            if inside(bmin, bmax, margin) {
                rows.push(CoverRow {
                    map: loc.name.clone(),
                    script,
                    label: format!("#{i}"),
                    model: match def {
                        Some(d) => format!(
                            "interior {:#010x} ({} entities, {} sets)",
                            e.archetype_hash,
                            d.entities.len(),
                            d.entity_sets.len()
                        ),
                        None => model_name(a),
                    },
                    level: if e.is_mlo_instance {
                        "MLO".to_string()
                    } else {
                        level_name(e.lod_level).to_string()
                    },
                    lod,
                    lod_from_arch: e.lod_dist <= 0.0,
                    child_lod: child_lod_dist(e, lod),
                    parent: format!(
                        "{}{}",
                        e.parent_index,
                        if e.lod_in_parent_ymap() {
                            "(parent map)"
                        } else {
                            ""
                        }
                    ),
                    children: e.num_children,
                    flags: e.flags,
                    aflags: a.flags,
                    pos: e.position,
                    bmin,
                    bmax,
                });
            }
            // An interior placement: its definition's entities (and the entity
            // sets it switches on) in world space, as the streamer places them.
            let Some(def) = def else {
                continue;
            };
            let instance_rot = e.orientation();
            let sets = def
                .entity_sets
                .iter()
                .filter(|s| e.default_entity_sets.contains(&s.name_hash))
                .flat_map(|s| s.entities.iter());
            for (j, ie) in def.entities.iter().chain(sets).enumerate() {
                let Some(ia) = db.get(ie.archetype_hash) else {
                    continue;
                };
                let (pos, rot) = interior_world(
                    e.position,
                    instance_rot,
                    ie.position,
                    stored_rotation_to_world(ie.rotation),
                );
                let (bmin, bmax) = world_box(ia, ie.scale_xy, ie.scale_z, rot, pos);
                if !inside(bmin, bmax, margin) {
                    continue;
                }
                rows.push(CoverRow {
                    map: loc.name.clone(),
                    script,
                    label: format!("#{i}/{j}"),
                    model: model_name(ia),
                    level: "HD (interior)".to_string(),
                    lod: if ie.lod_dist > 0.0 {
                        ie.lod_dist
                    } else {
                        ia.lod_dist
                    },
                    lod_from_arch: ie.lod_dist <= 0.0,
                    child_lod: 0.0,
                    parent: format!("placement #{i}"),
                    children: 0,
                    flags: ie.flags,
                    aflags: ia.flags,
                    pos,
                    bmin,
                    bmax,
                });
            }
        }
    }
    // Largest boxes first: terrain and buildings before the props on them.
    rows.sort_by(|a, b| {
        b.volume()
            .partial_cmp(&a.volume())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    println!(
        "{} entities in {} maps cover ({}, {}, {}) within {margin} m:",
        rows.len(),
        maps.len(),
        at.x,
        at.y,
        at.z
    );
    for r in &rows {
        println!(
            "{}{} {} {} {} lodDist={:.0}{} childLodDist={:.0} parent={} children={} flags={:#x} aflags={:#x} pos=({:.0},{:.0},{:.0}) box=({:.0},{:.0},{:.0})..({:.0},{:.0},{:.0})",
            r.map,
            if r.script { "(script)" } else { "" },
            r.label,
            r.model,
            r.level,
            r.lod,
            if r.lod_from_arch { "(arch)" } else { "" },
            r.child_lod,
            r.parent,
            r.children,
            r.flags,
            r.aflags,
            r.pos.x,
            r.pos.y,
            r.pos.z,
            r.bmin.x,
            r.bmin.y,
            r.bmin.z,
            r.bmax.x,
            r.bmax.y,
            r.bmax.z,
        );
    }
    Ok(())
}

/// One line of `cover`: a map entity, an interior placement, or an entity of
/// a placed interior (`label` `#i/j`).
struct CoverRow {
    map: String,
    script: bool,
    label: String,
    model: String,
    level: String,
    lod: f32,
    lod_from_arch: bool,
    child_lod: f32,
    parent: String,
    children: u32,
    flags: u32,
    /// The archetype's flags (bit 11: shadow proxy).
    aflags: u32,
    pos: Vec3,
    bmin: Vec3,
    bmax: Vec3,
}

impl CoverRow {
    fn volume(&self) -> f32 {
        (self.bmax.x - self.bmin.x) * (self.bmax.y - self.bmin.y) * (self.bmax.z - self.bmin.z)
    }
}

/// The world-space box of an archetype's bounding box placed with a scale,
/// a world rotation (x, y, z, w) and a position.
fn world_box(
    a: &ArchetypeRec,
    scale_xy: f32,
    scale_z: f32,
    rot: [f32; 4],
    pos: Vec3,
) -> (Vec3, Vec3) {
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for k in 0..8 {
        let corner = Vec3::new(
            if k & 1 == 0 { a.bb_min.x } else { a.bb_max.x } * scale_xy,
            if k & 2 == 0 { a.bb_min.y } else { a.bb_max.y } * scale_xy,
            if k & 4 == 0 { a.bb_min.z } else { a.bb_max.z } * scale_z,
        );
        let w = rotate(corner, rot) + pos;
        for (j, v) in [w.x, w.y, w.z].into_iter().enumerate() {
            min[j] = min[j].min(v);
            max[j] = max[j].max(v);
        }
    }
    (
        Vec3::new(min[0], min[1], min[2]),
        Vec3::new(max[0], max[1], max[2]),
    )
}

fn level_name(l: LodLevel) -> &'static str {
    match l {
        LodLevel::Hd => "HD",
        LodLevel::Lod => "LOD",
        LodLevel::Slod1 => "SLOD1",
        LodLevel::Slod2 => "SLOD2",
        LodLevel::Slod3 => "SLOD3",
        LodLevel::OrphanHd => "ORPHANHD",
        LodLevel::Slod4 => "SLOD4",
    }
}

fn content(n: &YmapNode) -> String {
    let names = [
        "HD",
        "LOD",
        "SLOD2+",
        "Interior",
        "SLOD",
        "Occl",
        "Phys",
        "LODlights",
        "Distlights",
        "Critical",
        "Grass",
    ];
    names
        .iter()
        .enumerate()
        .filter(|(i, _)| n.content_flags & (1 << i) != 0)
        .map(|(_, s)| *s)
        .collect::<Vec<_>>()
        .join(",")
}
