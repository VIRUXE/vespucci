//! Every `.rpf` under the game folder in the game's load order: base
//! archives, then `update.rpf`, then DLC packs in `dlclist.xml` order
//! stable-sorted by each pack's `setup2.xml` `<order>`. Later archives
//! override earlier ones. Carried over from rage-cli's `index.rs`
//! (VIRUXE, public domain), which mirrors CodeWalker's `InitDlcList`.

use crate::archive::Archive;
use anyhow::{bail, Context, Result};
use rpf_archive::{parse_dlc_list, parse_dlc_setup_order, GtaKeys};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn ranked_archives(game_root: &Path, keys: Option<&GtaKeys>) -> Result<Vec<PathBuf>> {
    let mut archives = collect_archives(game_root)?;
    if archives.is_empty() {
        bail!("no .rpf archives found under {}", game_root.display());
    }
    let dlc_rank = dlc_load_order(game_root, keys);
    archives.sort_by_key(|p| {
        let tier = archive_tier(p);
        let rank = match &tier {
            ArchiveTier::Dlc(name) => dlc_rank.get(name).copied().unwrap_or(u32::MAX),
            _ => 0,
        };
        (tier.rank(), rank, p.clone())
    });
    Ok(archives)
}

/// Every `.rpf` file under `path`, sorted; or `path` itself when it is a file.
pub fn collect_archives(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        bail!("{} does not exist", path.display());
    }
    let mut found = Vec::new();
    walk_dir(path, &mut found)?;
    found.sort();
    Ok(found)
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk_dir(&path, out)?;
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("rpf")) {
            out.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveTier {
    Base,
    Update,
    Dlc(String),
}

impl ArchiveTier {
    fn rank(&self) -> u8 {
        match self {
            ArchiveTier::Base => 0,
            ArchiveTier::Update => 1,
            ArchiveTier::Dlc(_) => 2,
        }
    }
}

pub fn archive_tier(path: &Path) -> ArchiveTier {
    let normalized = path.to_string_lossy().to_lowercase().replace('\\', "/");
    // Every dlcpacks archive lives under update/x64/dlcpacks, so test it first.
    if let Some(name) = dlc_pack_name(&normalized) {
        return ArchiveTier::Dlc(name);
    }
    if normalized.contains("/update/") {
        return ArchiveTier::Update;
    }
    ArchiveTier::Base
}

fn dlc_pack_name(normalized_path: &str) -> Option<String> {
    let after = normalized_path.split("/dlcpacks/").nth(1)?;
    let name = after.split('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Pack name -> load rank (0 loads first). Packs that cannot be placed are absent.
fn dlc_load_order(game_root: &Path, keys: Option<&GtaKeys>) -> HashMap<String, u32> {
    let update_rpf = game_root.join("update").join("update.rpf");
    let archive = match Archive::open(&update_rpf, keys) {
        Ok(a) => a,
        Err(err) => {
            log::debug!("no DLC load order ({} unreadable: {err})", update_rpf.display());
            return HashMap::new();
        }
    };
    if archive.require_keys(keys).is_err() {
        return HashMap::new();
    }
    let names = match archive.find_file("common/data/dlclist.xml").and_then(|f| archive.extract(f, keys).ok()) {
        Some(data) => parse_dlc_list(&data).unwrap_or_default(),
        None => return HashMap::new(),
    };
    let mut packs: Vec<(String, i32)> = Vec::with_capacity(names.len());
    for name in names {
        let dlc_rpf = game_root.join("update").join("x64").join("dlcpacks").join(&name).join("dlc.rpf");
        let order = Archive::open(&dlc_rpf, keys)
            .ok()
            .and_then(|pack| pack.find_file("setup2.xml").and_then(|f| pack.extract(f, keys).ok()))
            .and_then(|data| parse_dlc_setup_order(&data).ok())
            .unwrap_or(-1);
        packs.push((name, order));
    }
    rank_dlc_packs(packs)
}

/// Stable sort by `order` keeps `dlclist.xml` position as the tie-break,
/// mirroring CodeWalker's `DlcSetupFiles.OrderBy(o => o.order)`.
fn rank_dlc_packs(mut packs: Vec<(String, i32)>) -> HashMap<String, u32> {
    packs.sort_by_key(|(_, order)| *order);
    packs.into_iter().enumerate().map(|(rank, (name, _))| (name, rank as u32)).collect()
}
