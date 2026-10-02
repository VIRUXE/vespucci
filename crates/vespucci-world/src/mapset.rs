//! Which map files the game actually loads.
//!
//! Every DLC pack ships a `setup2.xml` (its change sets, grouped) and a
//! `content.xml` (its data files and what each change set enables or
//! invalidates). Base archives are active unless a change set invalidates
//! them; a pack's archives are active only once something enables them.
//! Story mode runs every group except `GROUP_MAP`, which the game executes
//! when entering Online (that is where the `hei_*` map variants come from).
//! This mirrors CodeWalker's `InitActiveMapRpfFiles`, with `disabled`
//! data files honoured and map change sets applied after all startup ones.
//! The set governs map placement (`.ymap`) only; archetype definitions
//! (`.ytyp`) stay registered from every archive, as in the game.

use anyhow::{Context, Result};
use std::collections::HashSet;
use vespucci_game::{FileLoc, GameFs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Story mode: base maps plus the packs' startup changes.
    SinglePlayer,
    /// Online: also the `GROUP_MAP` change sets.
    Multiplayer,
    /// Everything present in the files, duplicates included (CodeWalker's default view).
    All,
}

/// Which pack a file belongs to, and the path of its containing archive
/// relative to that pack (or to the base game), lowercase, no `x64/` prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Container {
    pack: Option<String>,
    key: String,
}

pub struct MapSet {
    pub mode: Mode,
    /// Per `GameFs::files` index.
    active: Vec<bool>,
    pub packs_read: usize,
    pub containers_enabled: usize,
    pub containers_invalidated: usize,
}

impl MapSet {
    pub fn all(fs: &GameFs) -> MapSet {
        MapSet {
            mode: Mode::All,
            active: vec![true; fs.files.len()],
            packs_read: 0,
            containers_enabled: 0,
            containers_invalidated: 0,
        }
    }

    pub fn build(fs: &GameFs, mode: Mode) -> Result<MapSet> {
        if mode == Mode::All {
            return Ok(Self::all(fs));
        }
        let t = std::time::Instant::now();
        let mut enabled: HashSet<Container> = HashSet::new();
        let mut invalidated: HashSet<String> = HashSet::new();
        let mut packs_read = 0;

        // Phase 1: every pack's data files and startup change sets, in load
        // order. Phase 2 (Online only): every pack's `GROUP_MAP` change sets,
        // which the game executes when entering Online, after all of phase 1.
        // Evaluating pack by pack instead would let a later patch pack's
        // overlay re-enable an archive an earlier pack's map change set removed.
        let mut phase_map: Vec<(String, String, HashSet<String>)> = Vec::new();
        for pack in fs.dlc_packs() {
            let Some(setup) = read_pack_file(fs, &pack, "setup2.xml") else {
                continue;
            };
            let setup =
                roxmltree::Document::parse(&setup).with_context(|| format!("{pack}/setup2.xml"))?;
            let dat_file = text(&setup, "datFile").unwrap_or_else(|| "content.xml".into());
            let mut startup_sets: HashSet<String> = HashSet::new();
            let mut map_sets: HashSet<String> = HashSet::new();
            for group in setup
                .descendants()
                .filter(|n| n.has_tag_name("contentChangeSetGroups"))
                .flat_map(|g| g.children().filter(|c| c.has_tag_name("Item")))
            {
                let name = group
                    .children()
                    .find(|c| c.has_tag_name("NameHash"))
                    .and_then(|c| c.text())
                    .unwrap_or("")
                    .trim()
                    .to_uppercase();
                let target = match name.as_str() {
                    "GROUP_MAP" => &mut map_sets,
                    "GROUP_ON_DEMAND" => continue,
                    _ => &mut startup_sets,
                };
                for cs in group
                    .descendants()
                    .filter(|c| c.has_tag_name("Item"))
                    .filter_map(|c| c.text())
                {
                    target.insert(cs.trim().to_uppercase());
                }
            }
            let Some(content_xml) = read_pack_file(fs, &pack, &dat_file) else {
                continue;
            };
            let content = roxmltree::Document::parse(&content_xml)
                .with_context(|| format!("{pack}/{dat_file}"))?;
            packs_read += 1;
            apply_pack(
                &pack,
                &content,
                &startup_sets,
                &mut enabled,
                &mut invalidated,
            );
            if mode == Mode::Multiplayer && !map_sets.is_empty() {
                phase_map.push((pack.clone(), content_xml, map_sets));
            }
        }
        for (pack, content_xml, map_sets) in &phase_map {
            let content = roxmltree::Document::parse(content_xml)
                .with_context(|| format!("{pack} content"))?;
            apply_pack(pack, &content, map_sets, &mut enabled, &mut invalidated);
        }

        let active = fs
            .files
            .iter()
            .map(|f| match container_of(fs, f) {
                None => true,
                Some(Container { pack: None, key }) => !invalidated.contains(&key),
                Some(c) => enabled.contains(&c),
            })
            .collect();
        let set = MapSet {
            mode,
            active,
            packs_read,
            containers_enabled: enabled.len(),
            containers_invalidated: invalidated.len(),
        };
        log::info!(
            "map set {:?}: {} packs, {} pack archives enabled, {} base archives invalidated, {} of {} files active, {:.1} s",
            mode,
            packs_read,
            set.containers_enabled,
            set.containers_invalidated,
            set.active.iter().filter(|a| **a).count(),
            fs.files.len(),
            t.elapsed().as_secs_f64()
        );
        Ok(set)
    }

    pub fn is_active(&self, file_index: usize) -> bool {
        self.active.get(file_index).copied().unwrap_or(true)
    }
}

/// Applies one pack's data files and the given change sets to the active set.
fn apply_pack(
    pack: &str,
    content: &roxmltree::Document,
    active_sets: &HashSet<String>,
    enabled: &mut HashSet<Container>,
    invalidated: &mut HashSet<String>,
) {
    // Data files mounted with the pack, unless marked disabled (then a change set enables them).
    for item in content
        .descendants()
        .filter(|n| n.has_tag_name("dataFiles"))
        .flat_map(|d| d.children().filter(|c| c.has_tag_name("Item")))
    {
        let ty = child_text(&item, "fileType").unwrap_or_default();
        if ty != "RPF_FILE" {
            continue;
        }
        let disabled = item
            .children()
            .find(|c| c.has_tag_name("disabled"))
            .and_then(|c| c.attribute("value"))
            == Some("true");
        if let Some(f) = child_text(&item, "filename") {
            if !disabled {
                enabled.insert(Container {
                    pack: Some(pack.to_string()),
                    key: key_of(&f),
                });
            }
        }
    }
    for cs in content
        .descendants()
        .filter(|n| n.has_tag_name("contentChangeSets"))
        .flat_map(|d| d.children().filter(|c| c.has_tag_name("Item")))
    {
        let name = child_text(&cs, "changeSetName")
            .unwrap_or_default()
            .to_uppercase();
        if !active_sets.contains(&name) {
            continue;
        }
        for f in list_items(&cs, "filesToEnable") {
            if f.ends_with(".rpf") {
                enabled.insert(Container {
                    pack: Some(pack.to_string()),
                    key: key_of(&f),
                });
            }
        }
        for map_cs in cs
            .children()
            .filter(|c| c.has_tag_name("mapChangeSetData"))
            .flat_map(|m| m.children().filter(|c| c.has_tag_name("Item")))
        {
            for f in list_items(&map_cs, "filesToInvalidate") {
                if f.ends_with(".rpf") {
                    let key = key_of(&f);
                    enabled.retain(|c| c.key != key || c.pack.as_deref() == Some(pack));
                    invalidated.insert(key);
                }
            }
            for f in list_items(&map_cs, "filesToEnable") {
                if f.ends_with(".rpf") {
                    enabled.insert(Container {
                        pack: Some(pack.to_string()),
                        key: key_of(&f),
                    });
                }
            }
        }
    }
}

/// A pack's own or patched (`update.rpf/dlc_patch/<pack>/`) copy of a file, patched preferred.
fn read_pack_file(fs: &GameFs, pack: &str, name: &str) -> Option<String> {
    for path in [
        format!("update/update.rpf/dlc_patch/{pack}/{name}"),
        format!("update/x64/dlcpacks/{pack}/dlc.rpf/{name}"),
    ] {
        if let Some(loc) = fs.get(&path) {
            if let Ok(data) = fs.read(loc) {
                return Some(String::from_utf8_lossy(&data).into_owned());
            }
        }
    }
    None
}

fn text(doc: &roxmltree::Document, tag: &str) -> Option<String> {
    doc.descendants()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(|s| s.trim().to_string())
}

fn child_text(node: &roxmltree::Node, tag: &str) -> Option<String> {
    node.children()
        .find(|c| c.has_tag_name(tag))
        .and_then(|c| c.text())
        .map(|s| s.trim().to_string())
}

fn list_items(node: &roxmltree::Node, tag: &str) -> Vec<String> {
    node.children()
        .filter(|c| c.has_tag_name(tag))
        .flat_map(|l| l.children().filter(|c| c.has_tag_name("Item")))
        .filter_map(|i| i.text())
        .map(|s| s.trim().to_lowercase())
        .collect()
}

/// `dlcMPHeist:/%PLATFORM%/levels/gta5/x/y.rpf` or `platform:/levels/gta5/x/y.rpf`
/// -> `levels/gta5/x/y.rpf`.
fn key_of(mounted: &str) -> String {
    let s = mounted
        .to_lowercase()
        .replace('\\', "/")
        .replace("%platform%", "x64");
    let rest = s.split_once(":/").map(|(_, r)| r).unwrap_or(&s);
    rest.strip_prefix("x64/")
        .unwrap_or(rest)
        .trim_start_matches('/')
        .to_string()
}

/// The pack and archive a file sits in, or `None` for files not inside a
/// nested archive (top-level entries are always active).
fn container_of(fs: &GameFs, f: &FileLoc) -> Option<Container> {
    let top = fs.archive_display(f.archive);
    let (pack, nested_base) = if let Some(rest) = top.strip_prefix("update/x64/dlcpacks/") {
        (Some(rest.split('/').next().unwrap_or("").to_string()), None)
    } else if top == "update/update.rpf"
        && f.nested
            .first()
            .is_some_and(|n| n.starts_with("dlc_patch/"))
    {
        let first = &f.nested[0];
        let mut parts = first.splitn(3, '/');
        let (_, pack, rest) = (parts.next(), parts.next()?, parts.next()?);
        (Some(pack.to_string()), Some(rest.to_string()))
    } else {
        (None, None)
    };
    let nested_last = f.nested.last()?;
    let key_path = if f.nested.len() == 1 {
        nested_base.unwrap_or_else(|| nested_last.clone())
    } else {
        nested_last.clone()
    };
    let key = key_path
        .strip_prefix("x64/")
        .unwrap_or(&key_path)
        .to_string();
    Some(Container { pack, key })
}
