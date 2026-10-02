use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Instant;

mod compare;
mod doctor;
mod files;
mod render;
mod render_model;
mod shader;
mod texture;
mod world;

#[derive(Parser)]
#[command(
    name = "vespucci",
    version,
    about = "GTA V map editor: desktop on Windows, headless on Linux"
)]
struct Cli {
    /// Game install directory (default: $GTAV_PATH)
    #[arg(long, env = "GTAV_PATH", global = true)]
    game: Option<PathBuf>,
    /// Log level: error, warn, info, debug, trace
    #[arg(long, env = "VESPUCCI_LOG", default_value = "info", global = true)]
    log: String,
    /// Which maps count as loaded: sp (story mode), mp (online), all (every file)
    #[arg(long, default_value = "sp", global = true)]
    mapset: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check the environment: game files, keys, Vulkan driver, DXVK; with --gpu, draw a test frame.
    Doctor {
        /// Create a D3D11 device, draw a triangle and read it back
        #[arg(long)]
        gpu: bool,
        /// Write the GPU test frame here as PNG
        #[arg(long)]
        out: Option<PathBuf>,
        /// Test frame size (square)
        #[arg(long, default_value_t = 256)]
        size: u32,
    },
    /// List game files. PATH is archive/dir, e.g. `x64c.rpf/levels/gta5/props`.
    Ls {
        path: Option<String>,
        /// Recurse into subdirectories and nested archives
        #[arg(short = 'R', long)]
        recursive: bool,
        /// Show sizes (disk, memory) and kind
        #[arg(short, long)]
        long: bool,
    },
    /// Extract one file (decrypted, decompressed) to stdout or --out.
    Cat {
        path: String,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Find files by glob, by exact name of a type, or by stem hash.
    Find {
        /// Glob on the file name, e.g. `prop_cs_heist_bag*`
        glob: Option<String>,
        /// File type without the dot, e.g. ydr
        #[arg(long)]
        ext: Option<String>,
        /// Exact name (with --ext): the file the game would load for it
        #[arg(long)]
        name: Option<String>,
        /// Stem hash in hex (with --ext)
        #[arg(long)]
        hash: Option<String>,
        #[arg(short, long)]
        long: bool,
    },
    /// Build the world index (archetypes, ymap tree) and report its size.
    Index,
    /// What a camera would stream at a position: ymaps in range, entity counts per LOD, model data size.
    Probe {
        /// Camera position X,Y,Z (game world units)
        #[arg(long, value_parser = parse_vec3)]
        pos: (f32, f32, f32),
        #[arg(long, default_value_t = 300.0)]
        radius: f32,
        #[arg(long)]
        json: bool,
    },
    /// Draw the world from a camera position with the game's own shaders to a PNG.
    Render {
        /// Camera position X,Y,Z
        #[arg(long, value_parser = parse_vec3)]
        pos: (f32, f32, f32),
        /// Point the camera looks at X,Y,Z
        #[arg(long, value_parser = parse_vec3)]
        look: (f32, f32, f32),
        #[arg(long, default_value_t = 50.0)]
        fov: f32,
        #[arg(long, default_value = "1280x720", value_parser = parse_size)]
        size: (u32, u32),
        /// Maps touching this radius around the camera are streamed
        #[arg(long, default_value_t = 300.0)]
        radius: f32,
        #[arg(long, default_value_t = 1.0)]
        lod_scale: f32,
        /// none (unlit) or basic (forward-lit preview)
        #[arg(long, default_value = "basic")]
        lighting: String,
        #[arg(long, default_value_t = 20000)]
        max_draws: usize,
        /// Stop loading models past this much geometry + texture data
        #[arg(long, default_value_t = 1500)]
        budget_mb: usize,
        /// Mip levels dropped near,mid,far (<100 m, <500 m, beyond)
        #[arg(long, default_value = "0,1,2")]
        mip_skip: String,
        #[arg(long)]
        flip_sun: bool,
        /// Scale on the linear HDR frame before the display curve
        #[arg(long, default_value_t = 1.0)]
        exposure: f32,
        /// Time of day HH:MM; picks the hour variants of time-dependent objects
        #[arg(long, default_value = "12:00")]
        time: String,
        /// Also show maps the game loads only on a script's request
        #[arg(long)]
        script_maps: bool,
        /// Print the model and material drawn at pixel X,Y (a second pass writes draw ids)
        #[arg(long, value_parser = parse_pixel)]
        pick: Option<(u32, u32)>,
        /// Also write a PNG with one colour per draw (same picking pass)
        #[arg(long)]
        id_map: Option<PathBuf>,
        #[arg(long, default_value = "frame.png")]
        out: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Draw one model with the game's own shaders to a PNG.
    RenderModel {
        /// Model: a name (prop_cs_heist_bag_01) or a game path (x64c.rpf/.../x.ydr)
        model: String,
        /// For .ydd/.yft: the entry name or hash to draw
        #[arg(long)]
        entry: Option<String>,
        /// Extra texture dictionaries (game path or name), earlier wins
        #[arg(long)]
        ytd: Vec<String>,
        #[arg(long, default_value = "high")]
        lod: String,
        /// Technique candidates, most wanted first (default: unlit_draw, draw)
        #[arg(long)]
        technique: Vec<String>,
        #[arg(long, default_value = "512x512", value_parser = parse_size)]
        size: (u32, u32),
        #[arg(long, default_value_t = 225.0)]
        yaw: f32,
        #[arg(long, default_value_t = 25.0)]
        pitch: f32,
        /// Upload matrices transposed (row-vector convention); the game's shaders take them as-is
        #[arg(long)]
        transpose: bool,
        /// Flip the preview sun direction (forward-lit techniques only)
        #[arg(long)]
        flip_sun: bool,
        /// Cull back faces (off: many game surfaces are single-sided)
        #[arg(long)]
        cull: bool,
        #[arg(long)]
        wireframe: bool,
        /// Drop this many top mip levels of every texture
        #[arg(long, default_value_t = 0)]
        mip_skip: u8,
        /// Palette row for `_tnt` materials (an entity's tintValue)
        #[arg(long, default_value_t = 0)]
        tint: u32,
        #[arg(long, default_value = "model.png")]
        out: PathBuf,
        /// Write the material binding report as JSON
        #[arg(long)]
        dump_binding: Option<PathBuf>,
    },
    /// Dump a texture dictionary (or a model's embedded textures) as PNG files.
    Texture {
        /// A .ytd/.ydr/.yft/.ydd name or game path
        what: String,
        #[arg(long, default_value = "textures")]
        out: PathBuf,
    },
    /// Compare two PNGs: PSNR and the share of pixels that differ.
    Compare {
        a: PathBuf,
        b: PathBuf,
        /// Fail (exit 1) when PSNR is below this
        #[arg(long)]
        min_psnr: Option<f64>,
    },
    /// Inspect one of the game's shader containers (.fxc).
    Shader {
        /// Shader name, e.g. normal_spec; or `--all` to check every shader
        #[arg(default_value = "")]
        name: String,
        /// Parse and reflect every shader in the game; report failures
        #[arg(long)]
        all: bool,
        /// List techniques and the shaders each pass uses (default when nothing else is asked)
        #[arg(long)]
        techniques: bool,
        /// List constant buffers, variables, defaults, textures and samplers
        #[arg(long)]
        vars: bool,
        /// Reflect every DXBC blob: bindings, cbuffer layouts, vertex inputs
        #[arg(long)]
        reflect: bool,
        /// Write one blob: STAGE:INDEX, e.g. vs:0 (needs --out)
        #[arg(long)]
        blob: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    // Die quietly when stdout is closed early (`vespucci ls | head`).
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    env_logger::Builder::new()
        .parse_filters(&cli.log)
        .format_timestamp(None)
        .init();
    let game = || {
        cli.game
            .as_deref()
            .context("game directory needed: --game DIR or $GTAV_PATH")
    };
    let mode = match cli.mapset.as_str() {
        "sp" => vespucci_world::Mode::SinglePlayer,
        "mp" => vespucci_world::Mode::Multiplayer,
        "all" => vespucci_world::Mode::All,
        other => anyhow::bail!("--mapset must be sp, mp or all, not {other}"),
    };
    match cli.cmd {
        Cmd::Doctor { gpu, out, size } => {
            doctor::run(cli.game.as_deref(), gpu, out.as_deref(), size)
        }
        Cmd::Ls {
            path,
            recursive,
            long,
        } => files::ls(
            &files::open(game()?)?,
            path.as_deref().unwrap_or(""),
            recursive,
            long,
        ),
        Cmd::Cat { path, out } => files::cat(&files::open(game()?)?, &path, out.as_deref()),
        Cmd::Find {
            glob,
            ext,
            name,
            hash,
            long,
        } => files::find(
            &files::open(game()?)?,
            ext.as_deref(),
            name.as_deref(),
            hash.as_deref(),
            glob.as_deref(),
            long,
        ),
        Cmd::Index => world::index(&files::open(game()?)?, mode),
        Cmd::Probe { pos, radius, json } => world::probe(
            &files::open(game()?)?,
            mode,
            vespucci_world::Vec3::new(pos.0, pos.1, pos.2),
            radius,
            json,
        ),
        Cmd::Render {
            pos,
            look,
            fov,
            size,
            radius,
            lod_scale,
            lighting,
            max_draws,
            budget_mb,
            mip_skip,
            flip_sun,
            exposure,
            time,
            script_maps,
            pick,
            id_map,
            out,
            json,
        } => render::run(
            &files::open(game()?)?,
            mode,
            pos,
            look,
            fov,
            size,
            radius,
            lod_scale,
            &lighting,
            max_draws,
            budget_mb,
            &mip_skip,
            flip_sun,
            exposure,
            &time,
            script_maps,
            pick,
            id_map.as_deref(),
            &out,
            json,
        ),
        Cmd::RenderModel {
            model,
            entry,
            ytd,
            lod,
            technique,
            size,
            yaw,
            pitch,
            transpose,
            flip_sun,
            cull,
            wireframe,
            mip_skip,
            tint,
            out,
            dump_binding,
        } => render_model::run(
            &files::open(game()?)?,
            &model,
            entry.as_deref(),
            &ytd,
            &lod,
            &technique,
            size,
            yaw,
            pitch,
            transpose,
            flip_sun,
            cull,
            wireframe,
            mip_skip,
            tint,
            &out,
            dump_binding.as_deref(),
        ),
        Cmd::Texture { what, out } => texture::run(&files::open(game()?)?, &what, &out),
        Cmd::Compare { a, b, min_psnr } => compare::run(&a, &b, min_psnr),
        Cmd::Shader {
            name,
            all,
            techniques,
            vars,
            reflect,
            blob,
            out,
        } => {
            let fs = files::open(game()?)?;
            if all {
                shader::check_all(&fs)
            } else {
                shader::run(
                    &fs,
                    &name,
                    techniques,
                    vars,
                    reflect,
                    blob.as_deref(),
                    out.as_deref(),
                )
            }
        }
    }
}

pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let file = std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// Timed section helper for doctor output.
pub fn timed<T>(label: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let t = Instant::now();
    let r = f();
    match &r {
        Ok(_) => println!("  {label}: {:.3} s", t.elapsed().as_secs_f64()),
        Err(e) => println!(
            "  {label}: FAILED after {:.3} s: {e}",
            t.elapsed().as_secs_f64()
        ),
    }
    r
}

fn parse_vec3(s: &str) -> std::result::Result<(f32, f32, f32), String> {
    let v: Vec<f32> = s
        .split(',')
        .map(|p| p.trim().parse::<f32>().map_err(|e| e.to_string()))
        .collect::<std::result::Result<_, _>>()?;
    if v.len() != 3 {
        return Err("expected X,Y,Z".into());
    }
    Ok((v[0], v[1], v[2]))
}

fn parse_pixel(s: &str) -> std::result::Result<(u32, u32), String> {
    let (x, y) = s.split_once(',').ok_or("expected X,Y")?;
    Ok((
        x.trim()
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?,
        y.trim()
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?,
    ))
}

fn parse_size(s: &str) -> std::result::Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WxH")?;
    Ok((
        w.parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?,
        h.parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?,
    ))
}
