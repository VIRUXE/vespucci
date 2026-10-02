//! [`GameFs`]: every file in every archive of the install, nested archives
//! included, as one flat table with last-archive-wins lookups.

use crate::archive::Archive;
use crate::keys;
use crate::load_order::ranked_archives;
use anyhow::{Context, Result};
use rayon::prelude::*;
use rpf_archive::{rage_joaat, GtaKeys};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where a file lives and what it is.
#[derive(Debug, Clone)]
pub struct FileLoc {
    /// Index into [`GameFs::archives`].
    pub archive: u32,
    /// Nested `.rpf` paths to descend through, each relative to the previous archive.
    pub nested: Vec<String>,
    /// Path inside the innermost archive, lowercase, `/`-separated.
    pub inner: String,
    /// Lowercase file name, and its extension without the dot.
    pub name: String,
    pub ext: String,
    /// `joaat(lowercase stem)` — the hash the game uses to refer to the file.
    pub stem_hash: u32,
    /// Bytes on disk (compressed) and in memory (uncompressed / resource pages).
    pub size: u32,
    pub mem_size: u32,
    pub is_resource: bool,
}

impl FileLoc {
    /// `x64c.rpf/levels/gta5/props/lev_des.rpf/prop_x.ydr`, the way the table is keyed.
    pub fn full_path(&self, fs: &GameFs) -> String {
        let mut s = fs.archive_display(self.archive);
        for n in &self.nested {
            s.push('/');
            s.push_str(n);
        }
        s.push('/');
        s.push_str(&self.inner);
        s
    }
}

pub struct GameFs {
    pub root: PathBuf,
    pub keys: GtaKeys,
    /// Top-level archives in load order (later overrides earlier).
    pub archives: Vec<PathBuf>,
    pub files: Vec<FileLoc>,
    by_path: HashMap<String, u32>,
    /// `(ext, stem_hash)` -> last file with that name; how the game resolves names.
    by_hash: HashMap<(String, u32), u32>,
}

impl GameFs {
    /// Recovers keys, ranks the archives and scans every table of contents.
    /// Archives are scanned in parallel and merged in rank order.
    pub fn open(root: &Path) -> Result<GameFs> {
        let (keys, _) = keys::load(root)?;
        let archives = ranked_archives(root, Some(&keys))?;
        let scanned: Vec<Result<Vec<FileLoc>>> = archives
            .par_iter()
            .enumerate()
            .map(|(i, path)| {
                let mut out = Vec::new();
                let archive = Archive::open(path, Some(&keys))
                    .with_context(|| format!("opening {}", path.display()))?;
                scan(&archive, i as u32, &[], Some(&keys), &mut out);
                Ok(out)
            })
            .collect();
        let mut files = Vec::new();
        for r in scanned {
            files.extend(r?);
        }
        let mut by_path = HashMap::with_capacity(files.len());
        let mut by_hash = HashMap::with_capacity(files.len());
        let mut fs = GameFs {
            root: root.to_path_buf(),
            keys,
            archives,
            files: Vec::new(),
            by_path: HashMap::new(),
            by_hash: HashMap::new(),
        };
        for (i, f) in files.iter().enumerate() {
            by_path.insert(f.full_path(&fs), i as u32);
            by_hash.insert((f.ext.clone(), f.stem_hash), i as u32);
        }
        fs.files = files;
        fs.by_path = by_path;
        fs.by_hash = by_hash;
        Ok(fs)
    }

    /// DLC pack directory names in load order (`mpheist`, `patchday2ng`, ...).
    pub fn dlc_packs(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for i in 0..self.archives.len() {
            let p = self.archive_display(i as u32);
            if let Some(rest) = p.strip_prefix("update/x64/dlcpacks/") {
                if let Some(name) = rest.split('/').next() {
                    if seen.insert(name.to_string()) {
                        out.push(name.to_string());
                    }
                }
            }
        }
        out
    }

    /// Archive path relative to the game root, `/`-separated, lowercase.
    pub fn archive_display(&self, index: u32) -> String {
        let p = &self.archives[index as usize];
        let rel = p.strip_prefix(&self.root).unwrap_or(p);
        rel.to_string_lossy().replace('\\', "/").to_lowercase()
    }

    pub fn get(&self, full_path: &str) -> Option<&FileLoc> {
        let key = full_path.replace('\\', "/").to_lowercase();
        self.by_path.get(&key).map(|&i| &self.files[i as usize])
    }

    /// The file the game would load for `name` (with or without extension) of type `ext`.
    pub fn by_name(&self, ext: &str, name: &str) -> Option<&FileLoc> {
        let stem = name.rsplit('/').next().unwrap_or(name);
        let stem = stem.strip_suffix(&format!(".{ext}")).unwrap_or(stem);
        self.by_hash(ext, rage_joaat(&stem.to_lowercase()))
    }

    pub fn by_hash(&self, ext: &str, stem_hash: u32) -> Option<&FileLoc> {
        self.by_hash
            .get(&(ext.to_lowercase(), stem_hash))
            .map(|&i| &self.files[i as usize])
    }

    /// The file's bytes: extracted, decrypted, decompressed; resources keep their RSC7 header.
    pub fn read(&self, loc: &FileLoc) -> Result<Vec<u8>> {
        let keys = Some(&self.keys);
        let top = Archive::open(&self.archives[loc.archive as usize], keys)?;
        let mut archive = top;
        for nested in &loc.nested {
            let file = archive
                .find_file(nested)
                .with_context(|| format!("nested archive {nested} not found"))?;
            archive = archive
                .open_nested(file, keys)
                .with_context(|| format!("opening nested {nested}"))?;
        }
        let file = archive
            .find_file(&loc.inner)
            .with_context(|| format!("{} not found", loc.inner))?;
        archive
            .extract(file, keys)
            .with_context(|| format!("extracting {}", loc.inner))
    }
}

fn scan(
    archive: &Archive,
    top: u32,
    nested: &[String],
    keys: Option<&GtaKeys>,
    out: &mut Vec<FileLoc>,
) {
    for file in archive.list_files() {
        let name = file.name.to_lowercase();
        let inner = file.path.replace('\\', "/").to_lowercase();
        if name.ends_with(".rpf") {
            match archive.open_nested(file, keys) {
                Ok(child) => {
                    let mut chain = nested.to_vec();
                    chain.push(inner);
                    scan(&child, top, &chain, keys, out);
                }
                Err(err) => log::debug!("skipping nested {}: {err}", file.path),
            }
            continue;
        }
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s, e.to_string()),
            None => (name.as_str(), String::new()),
        };
        out.push(FileLoc {
            archive: top,
            nested: nested.to_vec(),
            inner,
            stem_hash: rage_joaat(stem),
            name: name.clone(),
            ext,
            size: file.size,
            mem_size: file.mem_size,
            is_resource: file.is_resource,
        });
    }
}
