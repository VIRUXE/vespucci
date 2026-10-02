//! RPF keys from the user's own game executable, cached so a run costs
//! milliseconds instead of the seconds the recovery takes.

use anyhow::{bail, Context, Result};
use rpf_archive::GtaKeys;
use std::path::{Path, PathBuf};

/// `GTA5.exe` inside a game folder (or the path itself when it is a file).
pub fn resolve_exe(game: &Path) -> Result<PathBuf> {
    if game.is_file() {
        return Ok(game.to_path_buf());
    }
    let exe = game.join("GTA5.exe");
    if exe.is_file() {
        return Ok(exe);
    }
    bail!("no GTA5.exe in {}", game.display())
}

/// Keys for the game at `game`, from the cache when this build of the
/// executable has been seen before, recovered from the exe otherwise.
/// Returns the keys and whether they came from the cache.
pub fn load(game: &Path) -> Result<(GtaKeys, bool)> {
    let exe = resolve_exe(game)?;
    let Some(entry) = cache_entry(&exe) else {
        return Ok((GtaKeys::extract_from_exe(&exe, None)?, false));
    };
    if let Ok(keys) = GtaKeys::load_from_path(&entry) {
        return Ok((keys, true));
    }
    if std::fs::create_dir_all(&entry).is_err() {
        return Ok((GtaKeys::extract_from_exe(&exe, None)?, false));
    }
    let keys = GtaKeys::extract_from_exe(&exe, Some(&entry)).with_context(|| format!("recovering keys from {}", exe.display()))?;
    Ok((keys, false))
}

/// `~/.cache/vespucci/keys/<size>-<mtime>` — a game update can never be
/// served stale keys.
fn cache_entry(exe: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(exe).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(cache_root()?.join("keys").join(format!("{}-{modified}", meta.len())))
}

/// `$VESPUCCI_CACHE`, else `~/.cache/vespucci`.
pub fn cache_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("VESPUCCI_CACHE") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".cache").join("vespucci"))
}

/// The game's version as its executable states it ("1.0.3889.0"): the
/// `ProductVersion` value of the PE version resource, plain UTF-16 in the file.
pub fn exe_version(exe: &Path) -> Option<String> {
    let data = std::fs::read(exe).ok()?;
    let key: Vec<u8> = "ProductVersion".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let start = data.windows(key.len()).position(|w| w == key.as_slice())? + key.len();
    let mut units = Vec::new();
    let mut pos = start;
    while pos + 1 < data.len() && units.len() < 32 {
        let unit = u16::from_le_bytes([data[pos], data[pos + 1]]);
        pos += 2;
        if unit == 0 {
            if units.is_empty() {
                continue;
            }
            break;
        }
        units.push(unit);
    }
    let version = String::from_utf16(&units).ok()?;
    let plausible = !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.') && version.contains('.');
    plausible.then_some(version)
}
