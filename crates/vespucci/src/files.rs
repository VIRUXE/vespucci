//! `ls`, `cat`, `find`: the game's files as one tree across every archive.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::time::Instant;
use vespucci_game::GameFs;

pub fn open(game: &Path) -> Result<GameFs> {
    let t = Instant::now();
    let fs = GameFs::open(game)?;
    log::info!(
        "{} files across {} archives in {:.1} s{}",
        fs.files.len(),
        fs.archives.len(),
        t.elapsed().as_secs_f64(),
        peak_rss()
            .map(|mb| format!(", peak RSS {mb} MB"))
            .unwrap_or_default()
    );
    Ok(fs)
}

/// Peak resident set size of this process in MB (Linux only).
pub fn peak_rss() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// Lists files whose full path starts with `prefix` (all files when empty).
/// Without `recursive`, only the next path component is shown, like `ls`.
pub fn ls(fs: &GameFs, prefix: &str, recursive: bool, long: bool) -> Result<()> {
    let prefix = prefix.replace('\\', "/").to_lowercase();
    let prefix = prefix.trim_matches('/').to_string();
    let mut shown = std::collections::BTreeSet::new();
    let mut count = 0usize;
    for f in &fs.files {
        let full = f.full_path(fs);
        let rest = match strip_dir_prefix(&full, &prefix) {
            Some(r) => r,
            None => continue,
        };
        count += 1;
        if recursive {
            print_entry(&full, f.size, f.mem_size, f.is_resource, long);
        } else {
            let head = match rest.split_once('/') {
                Some((dir, _)) => format!("{dir}/"),
                None => rest.to_string(),
            };
            if shown.insert(head.clone()) {
                if head.ends_with('/') {
                    println!("{head}");
                } else {
                    print_entry(&head, f.size, f.mem_size, f.is_resource, long);
                }
            }
        }
    }
    if count == 0 {
        bail!("nothing under '{prefix}'");
    }
    Ok(())
}

fn strip_dir_prefix<'a>(full: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(full);
    }
    let rest = full.strip_prefix(prefix)?;
    rest.strip_prefix('/')
}

fn print_entry(name: &str, size: u32, mem: u32, res: bool, long: bool) {
    if long {
        println!(
            "{:>10} {:>10} {} {}",
            size,
            mem,
            if res { "rsc" } else { "bin" },
            name
        );
    } else {
        println!("{name}");
    }
}

pub fn cat(fs: &GameFs, path: &str, out: Option<&Path>) -> Result<()> {
    let loc = fs
        .get(path)
        .with_context(|| format!("'{path}' is not in the game files"))?;
    let data = fs.read(loc)?;
    match out {
        Some(p) => {
            std::fs::write(p, &data)?;
            eprintln!("wrote {} bytes to {}", data.len(), p.display());
        }
        None => {
            use std::io::Write;
            std::io::stdout().write_all(&data)?;
        }
    }
    Ok(())
}

/// Finds files by name glob (`*`/`?`), by exact name of a type, or by stem hash.
pub fn find(
    fs: &GameFs,
    ext: Option<&str>,
    name: Option<&str>,
    hash: Option<&str>,
    glob: Option<&str>,
    long: bool,
) -> Result<()> {
    if let Some(h) = hash {
        let h = u32::from_str_radix(h.trim_start_matches("0x"), 16).context("hash must be hex")?;
        let ext = ext.context("--hash needs --ext")?;
        match fs.by_hash(ext, h) {
            Some(f) => print_entry(&f.full_path(fs), f.size, f.mem_size, f.is_resource, long),
            None => bail!("no .{ext} with hash {h:#010x}"),
        }
        return Ok(());
    }
    if let (Some(ext), Some(name), None) = (ext, name, glob) {
        match fs.by_name(ext, name) {
            Some(f) => print_entry(&f.full_path(fs), f.size, f.mem_size, f.is_resource, long),
            None => bail!("no .{ext} named {name}"),
        }
        return Ok(());
    }
    let pattern = glob
        .or(name)
        .context("give --glob, --name with --ext, or --hash with --ext")?
        .to_lowercase();
    let mut n = 0;
    for f in &fs.files {
        if ext.is_some_and(|e| !f.ext.eq_ignore_ascii_case(e)) {
            continue;
        }
        if glob_match(&pattern, &f.name) {
            print_entry(&f.full_path(fs), f.size, f.mem_size, f.is_resource, long);
            n += 1;
        }
    }
    if n == 0 {
        bail!("no matches");
    }
    Ok(())
}

/// `*` any run, `?` one char; whole-name match.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}
