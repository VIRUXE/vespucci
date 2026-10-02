//! `vespucci shader NAME`: look inside one of the game's shader containers.

use anyhow::{bail, Context, Result};
use std::path::Path;
use vespucci_fxc::{BindKind, Dxbc, FxcFile, SHADER_DIRS, STAGE_NAMES};
use vespucci_game::GameFs;

/// Loads `NAME.fxc` from the most authoritative shader directory that has it.
pub fn load(fs: &GameFs, name: &str) -> Result<(String, FxcFile)> {
    let name = name.strip_suffix(".fxc").unwrap_or(name);
    for dir in SHADER_DIRS {
        let path = format!("{dir}/{name}.fxc");
        if let Some(loc) = fs.get(&path) {
            let data = fs.read(loc)?;
            return Ok((
                path.clone(),
                FxcFile::parse(&data).with_context(|| format!("parsing {path}"))?,
            ));
        }
    }
    bail!("no shader named {name}")
}

/// Parses and reflects every shader in the game's shader directories; reports failures.
pub fn check_all(fs: &GameFs) -> Result<()> {
    let t = std::time::Instant::now();
    let (mut files, mut blobs, mut failed) = (0, 0, 0);
    for dir in SHADER_DIRS {
        for loc in fs
            .files
            .iter()
            .filter(|f| f.ext == "fxc" && f.full_path(fs).starts_with(dir))
        {
            files += 1;
            let path = loc.full_path(fs);
            let result = fs
                .read(loc)
                .and_then(|d| FxcFile::parse(&d))
                .and_then(|fxc| {
                    for g in &fxc.groups {
                        for sh in g {
                            let dxbc = Dxbc::parse(&sh.dxbc)?;
                            dxbc.rdef()?;
                            dxbc.version().context("no version token")?;
                            blobs += 1;
                        }
                    }
                    for sh in &fxc.groups[0] {
                        Dxbc::parse(&sh.dxbc)?.input_signature()?;
                    }
                    Ok(())
                });
            if let Err(e) = result {
                failed += 1;
                println!("FAIL {path}: {e:#}");
            }
        }
    }
    println!(
        "{files} shader files, {blobs} DXBC blobs, {failed} failures, {:.1} s",
        t.elapsed().as_secs_f64()
    );
    if failed > 0 {
        bail!("{failed} shader files failed");
    }
    Ok(())
}

pub fn run(
    fs: &GameFs,
    name: &str,
    techniques: bool,
    vars: bool,
    reflect: bool,
    blob: Option<&str>,
    out: Option<&Path>,
) -> Result<()> {
    let (path, fxc) = load(fs, name)?;
    println!("{path}");
    println!(
        "vertex type {:#x}; shaders: {}",
        fxc.vertex_type,
        STAGE_NAMES
            .iter()
            .zip(&fxc.groups)
            .map(|(n, g)| format!("{n} {}", g.len()))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for p in &fxc.presets {
        println!("preset {} = {}", p.name, p.value);
    }

    if let Some(spec) = blob {
        let (stage, idx) = spec
            .split_once(':')
            .context("--blob STAGE:INDEX, e.g. vs:0")?;
        let si = STAGE_NAMES
            .iter()
            .position(|s| *s == stage)
            .context("stage is one of vs ps cs ds gs hs")?;
        let shader = fxc.groups[si]
            .get(idx.parse::<usize>()?)
            .context("no shader at that index")?;
        let out = out.context("--blob needs --out")?;
        std::fs::write(out, &shader.dxbc)?;
        println!(
            "wrote {} ({} bytes) to {}",
            shader.name,
            shader.dxbc.len(),
            out.display()
        );
        return Ok(());
    }

    if techniques || (!vars && !reflect) {
        println!("techniques:");
        for t in &fxc.techniques {
            for (pi, p) in t.passes.iter().enumerate() {
                let stages: Vec<String> = (0..6)
                    .filter_map(|s| {
                        fxc.pass_shader(p, s)
                            .map(|sh| format!("{}={}", STAGE_NAMES[s], sh.name))
                    })
                    .collect();
                println!("  {:<32} pass {pi}: {}", t.name, stages.join(" "));
            }
        }
    }

    if vars {
        println!("cbuffers:");
        for cb in fxc.cbuffers.iter().chain(&fxc.cbuffers2) {
            println!(
                "  {:<28} {:>5} B  slots vs{} ps{} cs{} ds{} gs{} hs{}",
                cb.name,
                cb.size,
                cb.slots[0],
                cb.slots[1],
                cb.slots[2],
                cb.slots[3],
                cb.slots[4],
                cb.slots[5]
            );
            for v in fxc
                .variables
                .iter()
                .filter(|v| v.cbuffer_hash == cb.name_hash)
            {
                println!(
                    "      {:<26} {:?}x{} @{:<3} param {:<24} default {:?}",
                    v.name,
                    v.ty,
                    v.count,
                    v.offset,
                    v.param_name,
                    v.default_floats()
                );
            }
        }
        println!("resources:");
        for v in &fxc.resources {
            let ann: Vec<String> = v
                .params
                .iter()
                .map(|(k, val)| format!("{k}={val:?}"))
                .collect();
            println!(
                "  {:<28} {:?} slot {:<2} group {} param {:<24} {}",
                v.name,
                v.ty,
                v.slot,
                v.group,
                v.param_name,
                ann.join(" ")
            );
        }
    }

    if reflect {
        for (si, group) in fxc.groups.iter().enumerate() {
            for (i, sh) in group.iter().enumerate() {
                let dxbc = Dxbc::parse(&sh.dxbc)?;
                let (maj, min, _) =
                    dxbc.version()
                        .unwrap_or((0, 0, vespucci_fxc::ProgramType::Unknown(0)));
                let rdef = dxbc.rdef()?;
                println!(
                    "{}:{i} {} (sm{maj}.{min}, {} bytes)",
                    STAGE_NAMES[si],
                    sh.name,
                    sh.dxbc.len()
                );
                for b in &rdef.bindings {
                    let kind = match b.kind {
                        BindKind::CBuffer => "cbuffer",
                        BindKind::TBuffer => "tbuffer",
                        BindKind::Texture => "texture",
                        BindKind::Sampler => "sampler",
                        BindKind::Uav => "uav",
                        BindKind::Structured => "structured",
                        BindKind::ByteAddress => "byteaddr",
                        BindKind::Other(_) => "other",
                    };
                    println!("    {kind:<10} {:<3} {}", b.bind_point, b.name);
                }
                for cb in &rdef.cbuffers {
                    println!(
                        "    cbuffer {} ({} B, {} vars)",
                        cb.name,
                        cb.size,
                        cb.vars.len()
                    );
                    for v in &cb.vars {
                        println!("        @{:<4} {:<4} {}", v.offset, v.size, v.name);
                    }
                }
                if si == 0 {
                    let isgn = dxbc.input_signature()?;
                    let ins: Vec<String> = isgn
                        .elements
                        .iter()
                        .map(|e| format!("{}{}", e.semantic, e.index))
                        .collect();
                    println!("    inputs: {}", ins.join(" "));
                }
            }
        }
    }
    Ok(())
}
