//! Every `.ymap` in the install with its extents and parent link: the tree
//! streaming walks to decide which maps a camera can see.
//!
//! Extents come from the game's own `*_cache_y.dat` files where they cover a
//! map, and from the map's own header otherwise (each such map is
//! decompressed once; the result is worth caching to disk later).

use crate::mapset::MapSet;
use anyhow::Result;
use rage_formats::{parse_cache_dat, parse_ymap_header, Vec3};
use rayon::prelude::*;
use std::collections::HashMap;
use std::time::Instant;
use vespucci_game::GameFs;

#[derive(Debug, Clone)]
pub struct YmapNode {
    pub name_hash: u32,
    pub parent_hash: u32,
    pub content_flags: u32,
    pub streaming_min: Vec3,
    pub streaming_max: Vec3,
    pub entities_min: Vec3,
    pub entities_max: Vec3,
    /// Index into `GameFs::files`.
    pub file: u32,
    pub from_cache: bool,
}

impl YmapNode {
    pub const CONTENT_HD: u32 = 1;
    pub const CONTENT_LOD: u32 = 2;
    pub const CONTENT_SLOD2: u32 = 4;
    pub const CONTENT_INTERIOR: u32 = 8;
    pub const CONTENT_SLOD: u32 = 16;
    pub const CONTENT_OCCLUSION: u32 = 32;
    pub const CONTENT_PHYSICS: u32 = 64;
    pub const CONTENT_LOD_LIGHTS: u32 = 128;
    pub const CONTENT_DISTANT_LIGHTS: u32 = 256;
    pub const CONTENT_CRITICAL: u32 = 512;
    pub const CONTENT_GRASS: u32 = 1024;

    /// Whether a sphere (world space) touches this map's entity extents.
    pub fn touches_sphere(&self, c: Vec3, r: f32) -> bool {
        let dx = (self.entities_min.x - c.x)
            .max(0.0)
            .max(c.x - self.entities_max.x);
        let dy = (self.entities_min.y - c.y)
            .max(0.0)
            .max(c.y - self.entities_max.y);
        let dz = (self.entities_min.z - c.z)
            .max(0.0)
            .max(c.z - self.entities_max.z);
        dx * dx + dy * dy + dz * dz <= r * r
    }
}

#[derive(Default)]
pub struct YmapTree {
    pub nodes: Vec<YmapNode>,
    pub by_hash: HashMap<u32, u32>,
    /// Parent index -> child indices.
    pub children: HashMap<u32, Vec<u32>>,
    pub from_cache: usize,
    pub from_headers: usize,
    pub failed: usize,
}

impl YmapTree {
    pub fn build(fs: &GameFs, set: &MapSet) -> Result<YmapTree> {
        let t = Instant::now();
        // Every map file, last in load order wins for a given name.
        let mut ymap_files: HashMap<u32, u32> = HashMap::new();
        for (i, f) in fs.files.iter().enumerate() {
            if f.ext == "ymap" && set.is_active(i) {
                ymap_files.insert(f.stem_hash, i as u32);
            }
        }

        // Cache files describe most maps' extents without opening them.
        let mut cached: HashMap<u32, rage_formats::MapDataNode> = HashMap::new();
        for f in fs.files.iter().filter(|f| f.name.ends_with("cache_y.dat")) {
            match fs.read(f).and_then(|d| parse_cache_dat(&d)) {
                Ok(dat) => {
                    for n in dat.map_nodes {
                        cached.insert(n.name, n);
                    }
                }
                Err(e) => log::debug!("{}: {e:#}", f.name),
            }
        }

        let mut tree = YmapTree::default();
        let mut uncovered = Vec::new();
        for (&hash, &file) in &ymap_files {
            match cached.get(&hash) {
                Some(n) => {
                    tree.from_cache += 1;
                    tree.nodes.push(YmapNode {
                        name_hash: n.name,
                        parent_hash: n.parent,
                        content_flags: n.content_flags,
                        streaming_min: n.streaming_min,
                        streaming_max: n.streaming_max,
                        entities_min: n.entities_min,
                        entities_max: n.entities_max,
                        file,
                        from_cache: true,
                    });
                }
                None => uncovered.push((hash, file)),
            }
        }

        let headers: Vec<Option<YmapNode>> = uncovered
            .par_iter()
            .map(|&(_, file)| {
                let loc = &fs.files[file as usize];
                match fs.read(loc).and_then(|d| parse_ymap_header(&d)) {
                    Ok(h) => Some(YmapNode {
                        name_hash: h.name_hash,
                        parent_hash: h.parent_hash,
                        content_flags: h.content_flags,
                        streaming_min: h.streaming_extents_min,
                        streaming_max: h.streaming_extents_max,
                        entities_min: h.entities_extents_min,
                        entities_max: h.entities_extents_max,
                        file,
                        from_cache: false,
                    }),
                    Err(e) => {
                        log::debug!("{}: {e:#}", loc.name);
                        None
                    }
                }
            })
            .collect();
        for h in headers {
            match h {
                Some(n) => {
                    tree.from_headers += 1;
                    tree.nodes.push(n);
                }
                None => tree.failed += 1,
            }
        }

        for (i, n) in tree.nodes.iter().enumerate() {
            tree.by_hash.insert(n.name_hash, i as u32);
        }
        for (i, n) in tree.nodes.iter().enumerate() {
            if n.parent_hash != 0 {
                if let Some(&p) = tree.by_hash.get(&n.parent_hash) {
                    tree.children.entry(p).or_default().push(i as u32);
                }
            }
        }
        log::info!(
            "{} ymaps ({} from cache files, {} from headers, {} failed) in {:.1} s",
            tree.nodes.len(),
            tree.from_cache,
            tree.from_headers,
            tree.failed,
            t.elapsed().as_secs_f64()
        );
        Ok(tree)
    }

    pub fn get(&self, name_hash: u32) -> Option<&YmapNode> {
        self.by_hash
            .get(&name_hash)
            .map(|&i| &self.nodes[i as usize])
    }

    /// Maps whose entity extents touch a sphere.
    pub fn touching(&self, centre: Vec3, radius: f32) -> Vec<&YmapNode> {
        self.nodes
            .iter()
            .filter(|n| n.touches_sphere(centre, radius))
            .collect()
    }
}
