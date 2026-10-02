//! `vespucci texture`: dump a texture dictionary, or a model's embedded
//! textures, as PNG files.

use anyhow::{Context, Result};
use rage_formats::{encode_image, parse_drawables, parse_ytd, to_rgba_image, DrawableKind, ImageFormat, YtdTexture};
use std::path::Path;
use vespucci_game::GameFs;

pub fn run(fs: &GameFs, what: &str, out_dir: &Path) -> Result<()> {
    let loc = fs.get(what).or_else(|| fs.by_name("ytd", what)).or_else(|| fs.by_name("ydr", what)).or_else(|| fs.by_name("yft", what)).or_else(|| fs.by_name("ydd", what)).with_context(|| format!("{what}: not found"))?;
    let data = fs.read(loc)?;
    let textures: Vec<YtdTexture> = match loc.ext.as_str() {
        "ytd" => parse_ytd(&data)?,
        ext => {
            let kind = DrawableKind::from_extension(ext).with_context(|| format!("{what}: not a ytd/ydr/ydd/yft"))?;
            parse_drawables(&data, kind)?.into_iter().flat_map(|d| d.drawable.shader_group.map(|s| s.textures).unwrap_or_default()).collect()
        }
    };
    std::fs::create_dir_all(out_dir)?;
    println!("{} ({} textures):", loc.full_path(fs), textures.len());
    for t in &textures {
        let path = out_dir.join(format!("{}.png", t.name));
        match to_rgba_image(t).and_then(|img| encode_image(&img, ImageFormat::Png, 90)) {
            Ok(png) => {
                std::fs::write(&path, png)?;
                println!("  {:<40} {}x{} {:?} {} mips -> {}", t.name, t.width, t.height, t.format, t.levels, path.display());
            }
            Err(e) => println!("  {:<40} {}x{} {:?}: {e:#}", t.name, t.width, t.height, t.format),
        }
    }
    Ok(())
}
