//! Every archetype the game knows, from every `.ytyp`, last archive wins.

use anyhow::Result;
use rage_formats::{parse_ytyp, Archetype, Vec3};
use rayon::prelude::*;
use std::collections::HashMap;
use std::time::Instant;
use vespucci_game::GameFs;

#[derive(Debug, Clone)]
pub struct ArchetypeRec {
    pub name_hash: u32,
    pub bb_min: Vec3,
    pub bb_max: Vec3,
    pub lod_dist: f32,
    pub texture_dict_hash: u32,
    pub drawable_dictionary_hash: u32,
    pub asset_name_hash: u32,
    pub asset_type: u32,
    pub is_mlo: bool,
    pub flags: u32,
    /// Hour bits for time-dependent archetypes (`CTimeArchetypeDef`).
    pub time_flags: Option<u32>,
    /// `joaat(stem)` of the `.ytyp` this came from.
    pub ytyp_hash: u32,
}

impl ArchetypeRec {
    pub fn bounding_sphere(&self) -> (Vec3, f32) {
        let c = Vec3::new((self.bb_min.x + self.bb_max.x) * 0.5, (self.bb_min.y + self.bb_max.y) * 0.5, (self.bb_min.z + self.bb_max.z) * 0.5);
        let dx = self.bb_max.x - c.x;
        let dy = self.bb_max.y - c.y;
        let dz = self.bb_max.z - c.z;
        (c, (dx * dx + dy * dy + dz * dz).sqrt())
    }

    /// The model file the game loads: `(ext, stem hash)`; drawable
    /// dictionaries hold the model as a member named by `asset_name_hash`.
    pub fn model_file(&self) -> (&'static str, u32) {
        let name = if self.asset_name_hash != 0 { self.asset_name_hash } else { self.name_hash };
        match self.asset_type {
            Archetype::ASSET_TYPE_FRAGMENT => ("yft", name),
            Archetype::ASSET_TYPE_DRAWABLEDICTIONARY => ("ydd", self.drawable_dictionary_hash),
            _ => ("ydr", name),
        }
    }
}

#[derive(Default)]
pub struct ArchetypeDb {
    pub by_hash: HashMap<u32, ArchetypeRec>,
    pub ytyps_parsed: usize,
    pub ytyps_failed: usize,
}

impl ArchetypeDb {
    /// Parses every `.ytyp` in the install in parallel and merges them in
    /// load order, so a DLC's redefinition of an archetype wins.
    pub fn build(fs: &GameFs) -> Result<ArchetypeDb> {
        let t = Instant::now();
        let mut locs: Vec<_> = fs.files.iter().enumerate().filter(|(_, f)| f.ext == "ytyp").collect();
        // `fs.files` is already in archive rank order; keep it stable for the merge.
        locs.sort_by_key(|(i, _)| *i);
        let parsed: Vec<(u32, Result<Vec<Archetype>>)> = locs
            .par_iter()
            .map(|(_, loc)| {
                let r = fs.read(loc).and_then(|d| parse_ytyp(&d)).map(|y| y.archetypes);
                (loc.stem_hash, r)
            })
            .collect();
        let mut db = ArchetypeDb::default();
        for (ytyp_hash, r) in parsed {
            match r {
                Ok(archs) => {
                    db.ytyps_parsed += 1;
                    for a in archs {
                        db.by_hash.insert(
                            a.name_hash,
                            ArchetypeRec {
                                name_hash: a.name_hash,
                                bb_min: a.bb_min,
                                bb_max: a.bb_max,
                                lod_dist: a.lod_dist,
                                texture_dict_hash: a.texture_dict_hash,
                                drawable_dictionary_hash: a.drawable_dictionary_hash,
                                asset_name_hash: a.asset_name_hash,
                                asset_type: a.asset_type,
                                flags: a.flags,
                                time_flags: a.time_flags,
                                is_mlo: a.is_mlo,
                                ytyp_hash,
                            },
                        );
                    }
                }
                Err(e) => {
                    db.ytyps_failed += 1;
                    log::debug!("ytyp {ytyp_hash:#010x}: {e:#}");
                }
            }
        }
        log::info!("{} archetypes from {} ytyps ({} failed) in {:.1} s", db.by_hash.len(), db.ytyps_parsed, db.ytyps_failed, t.elapsed().as_secs_f64());
        Ok(db)
    }

    pub fn get(&self, name_hash: u32) -> Option<&ArchetypeRec> {
        self.by_hash.get(&name_hash)
    }
}
