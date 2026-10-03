//! Which entities a camera sees: the maps touching its range, their
//! entities linked into the LOD tree, and the game's LOD rule walked from
//! the roots down (the rule CodeWalker's renderer applies, see `lod_dist`,
//! `child_lod_dist` and [`collect`]).

use crate::archetypes::{ArchetypeDb, ArchetypeRec};
use crate::entities::{parse_entities, stored_rotation_to_world, Entity, LodLevel};
use crate::ymaps::{YmapNode, YmapTree};
use anyhow::Result;
use rage_formats::ymap::rotate;
use rage_formats::Vec3;
use rayon::prelude::*;
use std::collections::HashMap;
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
    /// Maps outside the radius whose streaming extents hold the camera (the game would have them loaded).
    pub ymaps_streaming: usize,
    /// Ancestor maps read on top of those, because LOD parents live there.
    pub ymaps_parents: usize,
    /// Child maps read because a loaded parent is within its childLodDist but misses children.
    pub ymaps_children: usize,
    pub ymaps_failed: usize,
    pub entities_seen: usize,
    /// Root entities (no LOD parent) farther than their lodDist: their whole subtree is out.
    pub beyond_lod_dist: usize,
    /// Parents that handed over to their children.
    pub hidden_by_children: usize,
    /// Children drawn although they are beyond their own lodDist, because the parent handed over.
    pub forced_children: usize,
    /// Parents kept because not every child was loaded (a child map outside the radius).
    pub children_missing: usize,
    pub unknown_archetype: usize,
    /// Interior placements left out because `include_mlo_instances` is off.
    pub mlo_skipped: usize,
    /// Interior placements drawn, and the entities they contributed.
    pub mlo_instances: usize,
    pub mlo_entities: usize,
    /// Interior placements whose archetype has no `CMloArchetypeDef`.
    pub mlo_without_def: usize,
    /// Shadow and reflection proxies, which the game draws only into its
    /// shadow and reflection maps (see [`is_proxy`]).
    pub proxies_skipped: usize,
    pub time_hidden: usize,
    pub script_maps_skipped: usize,
    pub lod_dist_from_archetype: usize,
    pub instances: usize,
}

/// An entity's lodDist: its own, or the archetype's when it stores -1 (half of
/// all map entities do). Without an archetype the stored value stands.
pub fn lod_dist(e: &Entity, a: Option<&ArchetypeRec>) -> f32 {
    if e.lod_dist > 0.0 {
        e.lod_dist
    } else {
        a.map_or(e.lod_dist, |a| a.lod_dist)
    }
}

/// The distance under which an entity defers to its children: as stored, or
/// half its lodDist when the map stores -1. A stored 0 means "never", so a
/// parent then only steps aside when a child comes within its own lodDist.
pub fn child_lod_dist(e: &Entity, lod_dist: f32) -> f32 {
    if e.child_lod_dist < 0.0 {
        lod_dist * 0.5
    } else {
        e.child_lod_dist
    }
}

/// Whether an entity is a shadow or reflection proxy: stand-in geometry the
/// game draws only into its shadow map or reflection map, never into the
/// frame. CodeWalker's `RenderIsEntityFinalRender`: archetype `flags` bit 11
/// (2048) marks shadow proxies, and these exact `CEntityDef::flags` values
/// mark the reflection proxies it has catalogued (golf-course and house
/// water proxies, tree and tunnel reflection proxies, mirror-only emissives,
/// interior reflection shells such as `v_7_gc_reflectproxy`).
pub fn is_proxy(entity_flags: u32, archetype_flags: u32) -> bool {
    archetype_flags & 2048 != 0
        || matches!(
            entity_flags,
            135_790_592 | 135_790_593 | 672_661_504 | 536_870_912 | 35_127_296 | 39_321_602
        )
}

/// Where an interior's entity ends up: its MLO-local position rotated by the
/// instance and moved to it, and its rotation composed with the instance's
/// (the local rotation first, then the instance's), which is CodeWalker's
/// `MloInstanceData::UpdateEntity`.
pub fn interior_world(
    instance_pos: Vec3,
    instance_rot: [f32; 4],
    local_pos: Vec3,
    local_rot: [f32; 4],
) -> (Vec3, [f32; 4]) {
    (
        rotate(local_pos, instance_rot) + instance_pos,
        quat_mul(instance_rot, local_rot),
    )
}

/// Hamilton product `a ⊗ b` of two (x, y, z, w) quaternions: rotating by the
/// result applies `b` first, then `a`.
fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
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

/// A parsed map in the working set.
struct LoadedMap {
    /// Index in `YmapTree::nodes`.
    node: u32,
    file: u32,
    parent_hash: u32,
    entities: Vec<Entity>,
}

/// (map index, entity index) in the working set.
type Key = (usize, usize);

/// The LOD tree over the loaded maps: who is whose parent, and each parent's
/// children in the working set.
struct LodTree<'a> {
    maps: &'a [LoadedMap],
    parent: Vec<Vec<Option<Key>>>,
    children: Vec<Vec<Vec<Key>>>,
}

impl<'a> LodTree<'a> {
    /// The parent of an entity, the way CodeWalker links them: the entry at
    /// `parent_index` in the same map when that is a coarser LOD level and the
    /// entity does not flag its parent as living in the parent map; otherwise
    /// the entry at `parent_index` in the parent map; otherwise none (a root).
    fn resolve_parent(
        maps: &[LoadedMap],
        by_hash: &HashMap<u32, usize>,
        m: usize,
        e: &Entity,
    ) -> Option<Key> {
        if e.parent_index < 0 {
            return None;
        }
        let pi = e.parent_index as usize;
        let same = &maps[m].entities;
        if !e.lod_in_parent_ymap() && pi < same.len() {
            let p = &same[pi];
            let coarser =
                p.lod_level as u32 > e.lod_level as u32 && p.lod_level != LodLevel::OrphanHd;
            if coarser {
                return Some((m, pi));
            }
        }
        let pm = *by_hash.get(&maps[m].parent_hash)?;
        (pi < maps[pm].entities.len()).then_some((pm, pi))
    }

    fn build(maps: &'a [LoadedMap], by_hash: &HashMap<u32, usize>) -> LodTree<'a> {
        let mut parent: Vec<Vec<Option<Key>>> = Vec::with_capacity(maps.len());
        let mut children: Vec<Vec<Vec<Key>>> = maps
            .iter()
            .map(|m| vec![Vec::new(); m.entities.len()])
            .collect();
        for (m, map) in maps.iter().enumerate() {
            let mut row = Vec::with_capacity(map.entities.len());
            for (i, e) in map.entities.iter().enumerate() {
                let p = Self::resolve_parent(maps, by_hash, m, e);
                if let Some((pm, pi)) = p {
                    children[pm][pi].push((m, i));
                }
                row.push(p);
            }
            parent.push(row);
        }
        LodTree {
            maps,
            parent,
            children,
        }
    }

    fn entity(&self, k: Key) -> &'a Entity {
        &self.maps[k.0].entities[k.1]
    }
}

/// Everything the walk needs besides the tree.
struct Walk<'a> {
    fs: &'a GameFs,
    db: &'a ArchetypeDb,
    camera: Vec3,
    opts: StreamOptions,
    trace_filter: Option<String>,
    stats: StreamStats,
    out: Vec<Instance>,
}

impl Walk<'_> {
    fn model_name(&self, a: Option<&ArchetypeRec>, e: &Entity) -> String {
        match a {
            Some(a) => {
                let (ext, h) = a.model_file();
                self.fs
                    .by_hash(ext, h)
                    .map(|l| l.name.clone())
                    .unwrap_or_else(|| format!("{h:#010x}.{ext}"))
            }
            None => format!("archetype {:#010x}", e.archetype_hash),
        }
    }

    /// Debug: VESPUCCI_TRACE_ENTITY=substring logs the LOD decision of matching entities.
    fn trace(&self, tree: &LodTree, k: Key, d: f32, lod: f32, decision: &str) {
        let Some(filter) = self.trace_filter.as_deref() else {
            return;
        };
        let e = tree.entity(k);
        let a = self.db.get(e.archetype_hash);
        let name = self.model_name(a, e);
        if !name.to_lowercase().contains(filter) {
            return;
        }
        let map_name = |m: usize| self.fs.files[tree.maps[m].file as usize].name.as_str();
        // The resolved parent, or why there is none.
        let parent = match tree.parent[k.0][k.1] {
            Some((pm, pi)) => format!("{pi}@{}", map_name(pm)),
            None if e.parent_index < 0 => "none".to_string(),
            None => format!(
                "{}(unlinked{}; parent map {})",
                e.parent_index,
                if e.lod_in_parent_ymap() {
                    ", in parent map"
                } else {
                    ""
                },
                self.fs
                    .by_hash("ymap", tree.maps[k.0].parent_hash)
                    .map(|l| l.name.clone())
                    .unwrap_or_else(|| format!("{:#010x}", tree.maps[k.0].parent_hash))
            ),
        };
        // Linked children as index@map, so a parent's incomplete list can be checked.
        let kids: Vec<String> = tree.children[k.0][k.1]
            .iter()
            .map(|&(cm, ci)| format!("#{ci}@{}", map_name(cm)))
            .collect();
        log::info!(
            "entity #{} {name} ymap {} d={d:.0} lodDist={lod:.0}{} childLodDist={:.0} children={}/{} level={:?} parent={parent} tint={} pos=({:.0},{:.0},{:.0}) -> {decision}{}",
            k.1,
            map_name(k.0),
            if e.lod_dist > 0.0 { "" } else { "(arch)" },
            child_lod_dist(e, lod),
            kids.len(),
            e.num_children,
            e.lod_level,
            e.tint,
            e.position.x,
            e.position.y,
            e.position.z,
            if kids.is_empty() {
                String::new()
            } else {
                format!(" kids=[{}]", kids.join(" "))
            }
        );
    }

    /// The LOD rule from an entity down. A parent steps aside for its children
    /// when every child is loaded and either the camera is within its
    /// childLodDist or some child is within its own lodDist; the children are
    /// then drawn whatever their own lodDist says (`forced`). Otherwise the
    /// entity itself is a leaf and is drawn.
    fn visit(&mut self, tree: &LodTree, k: Key, forced: bool) {
        let e = tree.entity(k);
        let a = self.db.get(e.archetype_hash);
        let scale = self.opts.lod_scale;
        let d = dist(e.position, self.camera);
        let lod = lod_dist(e, a);
        let kids = &tree.children[k.0][k.1];
        // Children Vespucci cannot draw (interior instances when interiors are left
        // out, entities without an archetype) count as not loaded: the game only
        // hides this parent because it draws them. Time-hidden children do count; the
        // game hides those without bringing the parent back.
        let loaded = kids
            .iter()
            .filter(|&&c| {
                let ce = tree.entity(c);
                !(ce.is_mlo_instance && !self.opts.include_mlo_instances)
                    && self.db.get(ce.archetype_hash).is_some()
            })
            .count();
        let complete = e.num_children > 0 && loaded >= e.num_children as usize;
        if e.num_children > 0 && !complete {
            self.stats.children_missing += 1;
            if d <= child_lod_dist(e, lod) * scale && log::log_enabled!(log::Level::Debug) {
                log::debug!(
                    "parent {} in {} ({:?}, d={d:.0}, childLodDist={:.0}) has {}/{} children loaded; drawn itself",
                    self.model_name(a, e),
                    self.fs.files[tree.maps[k.0].file as usize].name,
                    e.lod_level,
                    child_lod_dist(e, lod),
                    kids.len(),
                    e.num_children
                );
            }
        }
        let hand_over = complete
            && (d <= child_lod_dist(e, lod) * scale
                || kids.iter().any(|&c| {
                    let ce = tree.entity(c);
                    dist(ce.position, self.camera)
                        <= lod_dist(ce, self.db.get(ce.archetype_hash)) * scale
                }));
        if hand_over {
            self.stats.hidden_by_children += 1;
            self.trace(tree, k, d, lod, "children");
            for &c in kids {
                self.visit(tree, c, true);
            }
            return;
        }
        let beyond = d > lod * scale;
        if forced && beyond {
            self.stats.forced_children += 1;
        }
        let kind = match (forced, beyond, complete) {
            (true, true, _) => "leaf (forced beyond lodDist)",
            (_, _, false) if e.num_children > 0 => "leaf (children not all loaded)",
            _ => "leaf",
        };
        match self.emit(tree, k, e, a, d, lod) {
            Ok(()) if e.is_mlo_instance => self.trace(tree, k, d, lod, "interior"),
            Ok(()) => self.trace(tree, k, d, lod, kind),
            Err(why) => self.trace(tree, k, d, lod, &format!("{kind}, not drawn: {why}")),
        }
    }

    /// Adds a leaf to the output, or says why not.
    fn emit(
        &mut self,
        tree: &LodTree,
        k: Key,
        e: &Entity,
        a: Option<&ArchetypeRec>,
        d: f32,
        lod: f32,
    ) -> Result<(), &'static str> {
        if e.is_mlo_instance {
            if !self.opts.include_mlo_instances {
                self.stats.mlo_skipped += 1;
                return Err("interior instance");
            }
            return self.emit_interior(tree, k, e);
        }
        let Some(a) = a else {
            self.stats.unknown_archetype += 1;
            return Err("unknown archetype");
        };
        if e.lod_dist <= 0.0 {
            self.stats.lod_dist_from_archetype += 1;
        }
        if is_proxy(e.flags, a.flags) {
            self.stats.proxies_skipped += 1;
            return Err("shadow/reflection proxy");
        }
        if let (Some(h), Some(t)) = (self.opts.hour, a.time_flags) {
            if (t >> (h % 24)) & 1 == 0 {
                self.stats.time_hidden += 1;
                return Err("hidden at this hour");
            }
        }
        self.out.push(Instance {
            archetype: e.archetype_hash,
            position: e.position,
            orientation: e.orientation(),
            scale_xy: e.scale_xy,
            scale_z: e.scale_z,
            distance: d,
            lod_level: e.lod_level,
            lod_dist: lod,
            tint: e.tint,
            ymap: tree.maps[k.0].file,
            flags: e.flags,
        });
        Ok(())
    }

    /// An interior placement: the MLO's own entities plus those of the entity
    /// sets the placement switches on, moved into world space by the instance
    /// transform. They are all drawn while the instance is (CodeWalker's rule;
    /// the game also culls them by room and portal, which is not done yet).
    fn emit_interior(&mut self, tree: &LodTree, k: Key, e: &Entity) -> Result<(), &'static str> {
        let db = self.db;
        let Some(def) = db.mlo(e.archetype_hash) else {
            self.stats.mlo_without_def += 1;
            return Err("interior without a definition");
        };
        self.stats.mlo_instances += 1;
        let instance_rot = e.orientation();
        let ymap = tree.maps[k.0].file;
        let sets = def
            .entity_sets
            .iter()
            .filter(|s| e.default_entity_sets.contains(&s.name_hash))
            .flat_map(|s| s.entities.iter());
        for ie in def.entities.iter().chain(sets) {
            let Some(a) = db.get(ie.archetype_hash) else {
                self.stats.unknown_archetype += 1;
                continue;
            };
            if is_proxy(ie.flags, a.flags) {
                self.stats.proxies_skipped += 1;
                continue;
            }
            if let (Some(h), Some(t)) = (self.opts.hour, a.time_flags) {
                if (t >> (h % 24)) & 1 == 0 {
                    self.stats.time_hidden += 1;
                    continue;
                }
            }
            let (position, orientation) = interior_world(
                e.position,
                instance_rot,
                ie.position,
                stored_rotation_to_world(ie.rotation),
            );
            self.out.push(Instance {
                archetype: ie.archetype_hash,
                position,
                orientation,
                scale_xy: ie.scale_xy,
                scale_z: ie.scale_z,
                distance: dist(position, self.camera),
                lod_level: LodLevel::Hd,
                lod_dist: if ie.lod_dist > 0.0 {
                    ie.lod_dist
                } else {
                    a.lod_dist
                },
                tint: ie.tint,
                ymap,
                flags: ie.flags,
            });
            self.stats.mlo_entities += 1;
        }
        Ok(())
    }
}

pub fn collect(
    fs: &GameFs,
    tree: &YmapTree,
    db: &ArchetypeDb,
    camera: Vec3,
    opts: StreamOptions,
) -> Result<(Vec<Instance>, StreamStats)> {
    let mut stats = StreamStats::default();
    let stem_of = |file: u32| -> String {
        fs.files[file as usize]
            .name
            .trim_end_matches(".ymap")
            .to_lowercase()
    };
    // Time-of-day map variants: which suffixes exist per base name, over every map.
    let mut variants: HashMap<String, Vec<&'static str>> = HashMap::new();
    for n in &tree.nodes {
        let stem = stem_of(n.file);
        if let Some((base, v)) = time_variant(&stem) {
            let key = format!("{base}{}", if stem.ends_with("_lod") { "_lod" } else { "" });
            let v: &'static str = ["morning", "day", "evening", "night"]
                .into_iter()
                .find(|x| *x == v)
                .unwrap();
            variants.entry(key).or_default().push(v);
        }
    }

    // Reads and filters a batch of maps (in parallel), appending the kept ones.
    let load = |nodes: &[&YmapNode], maps: &mut Vec<LoadedMap>, stats: &mut StreamStats| {
        let parsed: Vec<(&YmapNode, Result<crate::entities::YmapEntities>)> = nodes
            .par_iter()
            .map(|n| {
                (
                    *n,
                    fs.read(&fs.files[n.file as usize])
                        .and_then(|d| parse_entities(&d)),
                )
            })
            .collect();
        for (n, r) in parsed {
            let file = n.file;
            match r {
                Ok(p) => {
                    log::debug!(
                        "ymap {} flags={:#x} content={:#x} entities={}",
                        fs.files[file as usize].name,
                        p.flags,
                        p.content_flags,
                        p.entities.len()
                    );
                    if p.flags & 1 != 0 && !opts.include_script_maps {
                        // Script-requested map: only the time-of-day variants are predictable.
                        let stem = stem_of(file);
                        let keep = match (time_variant(&stem), opts.hour) {
                            (Some((base, v)), Some(h)) => {
                                let key = format!(
                                    "{base}{}",
                                    if stem.ends_with("_lod") { "_lod" } else { "" }
                                );
                                wanted_variant(
                                    h,
                                    variants.get(&key).map(|v| v.as_slice()).unwrap_or(&[]),
                                ) == v
                            }
                            _ => false,
                        };
                        if !keep {
                            stats.script_maps_skipped += 1;
                            continue;
                        }
                    }
                    stats.entities_seen += p.entities.len();
                    maps.push(LoadedMap {
                        node: tree.index_of(n.name_hash).unwrap_or(u32::MAX),
                        file,
                        parent_hash: n.parent_hash,
                        entities: p.entities,
                    });
                }
                Err(e) => {
                    stats.ymaps_failed += 1;
                    log::debug!("{}: {e:#}", fs.files[file as usize].name);
                }
            }
        }
    };

    // The working set: maps in range, maps the game would have streamed for this
    // camera (streaming extents), and all their ancestors, because a map's LOD
    // parents live in its parent map and a map is not linked until its parent
    // is (CodeWalker does the same).
    let touching = tree.touching(camera, opts.radius);
    stats.ymaps_touched = touching.len();
    let mut have: HashMap<u32, ()> = touching.iter().map(|n| (n.name_hash, ())).collect();
    let mut queue = touching;
    // Debug: VESPUCCI_NO_STREAMING_EXTENTS=1 loads only the maps in --radius (plus parents).
    if std::env::var_os("VESPUCCI_NO_STREAMING_EXTENTS").is_none() {
        for n in tree.streaming_at(camera) {
            if have.insert(n.name_hash, ()).is_none() {
                stats.ymaps_streaming += 1;
                queue.push(n);
            }
        }
    }
    let mut nodes: Vec<&YmapNode> = Vec::with_capacity(queue.len());
    while let Some(n) = queue.pop() {
        nodes.push(n);
        if n.parent_hash != 0 {
            if let Some(p) = tree.get(n.parent_hash) {
                if have.insert(p.name_hash, ()).is_none() {
                    stats.ymaps_parents += 1;
                    queue.push(p);
                }
            }
        }
    }
    let mut maps: Vec<LoadedMap> = Vec::with_capacity(nodes.len());
    load(&nodes, &mut maps, &mut stats);

    // A parent only steps aside when every child is loaded. For parents the
    // camera is within childLodDist of, pull in the child maps that hold the
    // rest of their children (the game streams them for the same reason),
    // until nothing more is needed.
    let link = |maps: &[LoadedMap]| -> (
        HashMap<u32, usize>,
        Vec<Vec<Option<Key>>>,
        Vec<Vec<Vec<Key>>>,
    ) {
        let by_hash: HashMap<u32, usize> = maps
            .iter()
            .enumerate()
            .map(|(i, m)| (fs.files[m.file as usize].stem_hash, i))
            .collect();
        let t = LodTree::build(maps, &by_hash);
        (by_hash, t.parent, t.children)
    };
    let mut by_hash;
    loop {
        let children;
        (by_hash, _, children) = link(&maps);
        let mut wanted: Vec<&YmapNode> = Vec::new();
        for (m, map) in maps.iter().enumerate() {
            if map.node == u32::MAX {
                continue;
            }
            let needs_children = map.entities.iter().enumerate().any(|(i, e)| {
                e.num_children > 0
                    && children[m][i].len() < e.num_children as usize
                    && dist(e.position, camera)
                        <= child_lod_dist(e, lod_dist(e, db.get(e.archetype_hash))) * opts.lod_scale
            });
            if !needs_children {
                continue;
            }
            for &c in tree.child_maps(map.node) {
                let n = &tree.nodes[c as usize];
                if have.insert(n.name_hash, ()).is_none() {
                    wanted.push(n);
                }
            }
        }
        if wanted.is_empty() {
            break;
        }
        stats.ymaps_children += wanted.len();
        load(&wanted, &mut maps, &mut stats);
    }

    // Link the LOD tree and walk it from the roots.
    let lod_tree = LodTree::build(&maps, &by_hash);
    let mut walk = Walk {
        fs,
        db,
        camera,
        opts,
        trace_filter: std::env::var("VESPUCCI_TRACE_ENTITY")
            .ok()
            .map(|s| s.to_lowercase()),
        stats,
        out: Vec::new(),
    };
    for (m, map) in maps.iter().enumerate() {
        for (i, e) in map.entities.iter().enumerate() {
            if lod_tree.parent[m][i].is_some() {
                continue;
            }
            let a = db.get(e.archetype_hash);
            let d = dist(e.position, camera);
            let lod = lod_dist(e, a);
            if d > lod * opts.lod_scale {
                walk.stats.beyond_lod_dist += 1;
                walk.trace(&lod_tree, (m, i), d, lod, "root beyond lodDist");
                continue;
            }
            walk.visit(&lod_tree, (m, i), false);
        }
    }
    let Walk {
        mut stats, mut out, ..
    } = walk;
    stats.instances = out.len();
    // Nearest first; ties broken by data, never by the order the maps were
    // read in, so the list is the same from run to run (#14).
    out.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.archetype.cmp(&b.archetype))
            .then(a.position.x.to_bits().cmp(&b.position.x.to_bits()))
            .then(a.position.y.to_bits().cmp(&b.position.y.to_bits()))
            .then(a.position.z.to_bits().cmp(&b.position.z.to_bits()))
    });
    Ok((out, stats))
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec3, x: f32, y: f32, z: f32) -> bool {
        (a.x - x).abs() < 1e-4 && (a.y - y).abs() < 1e-4 && (a.z - z).abs() < 1e-4
    }

    /// An interior entity sits where the instance's rotation and position put
    /// its MLO-local placement, and its own rotation is applied before the
    /// instance's (issue #19).
    #[test]
    fn interior_entities_follow_the_instance_transform() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let quarter_z = [0.0, 0.0, h, h];
        let quarter_x = [h, 0.0, 0.0, h];
        let identity = [0.0, 0.0, 0.0, 1.0];
        let (p, r) = interior_world(
            Vec3::new(10.0, 0.0, 5.0),
            quarter_z,
            Vec3::new(1.0, 0.0, 0.0),
            identity,
        );
        assert!(near(p, 10.0, 1.0, 5.0), "{p:?}");
        assert!(near(rotate(Vec3::new(1.0, 0.0, 0.0), r), 0.0, 1.0, 0.0));
        // Local first (a quarter turn about X leaves +X alone), then the instance's
        // quarter turn about Z: +X ends up at +Y, not at +Z.
        let (_, r) = interior_world(
            Vec3::new(0.0, 0.0, 0.0),
            quarter_z,
            Vec3::new(0.0, 0.0, 0.0),
            quarter_x,
        );
        assert!(near(rotate(Vec3::new(1.0, 0.0, 0.0), r), 0.0, 1.0, 0.0));
        assert!(near(rotate(Vec3::new(0.0, 1.0, 0.0), r), 0.0, 0.0, 1.0));
    }
}
