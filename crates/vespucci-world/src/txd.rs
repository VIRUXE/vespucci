//! Texture dictionary parents: when a model's own dictionary lacks a
//! texture, the game looks in the archetype dictionary's parent chain
//! (`gtxd.ymt`/`gtxd.meta`, `mph4_gtxd.ymt`, `vehicles.meta`).

use anyhow::Result;
use rage_formats::parse_txd_relationships;
use std::collections::HashMap;
use vespucci_game::{joaat, GameFs};

const FILES: [&str; 4] = ["gtxd.ymt", "gtxd.meta", "mph4_gtxd.ymt", "vehicles.meta"];

#[derive(Default)]
pub struct TxdParents {
    /// child dictionary hash -> parent dictionary hash; the first relationship seen wins.
    parents: HashMap<u32, u32>,
    pub files_read: usize,
}

impl TxdParents {
    pub fn build(fs: &GameFs) -> Result<TxdParents> {
        let mut t = TxdParents::default();
        for f in fs.files.iter().filter(|f| FILES.contains(&f.name.as_str())) {
            match fs.read(f).and_then(|d| parse_txd_relationships(&d)) {
                Ok(rels) => {
                    t.files_read += 1;
                    for r in rels {
                        t.parents.entry(joaat(&r.child.to_lowercase())).or_insert_with(|| joaat(&r.parent.to_lowercase()));
                    }
                }
                Err(e) => log::debug!("{}: {e:#}", f.full_path(fs)),
            }
        }
        log::info!("{} texture-parent relationships from {} files", t.parents.len(), t.files_read);
        Ok(t)
    }

    /// The dictionary and its ancestors, nearest first (cycle-guarded).
    pub fn chain(&self, txd: u32) -> Vec<u32> {
        let mut out = vec![txd];
        let mut cur = txd;
        while let Some(&p) = self.parents.get(&cur) {
            if out.contains(&p) || out.len() > 32 {
                break;
            }
            out.push(p);
            cur = p;
        }
        out
    }

    pub fn len(&self) -> usize {
        self.parents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parents.is_empty()
    }
}
