//! `vespucci compare`: PSNR between two images of the same size.

use anyhow::{bail, Context, Result};
use std::path::Path;

pub fn load_rgba(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(path).with_context(|| path.display().to_string())?,
    ));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let bytes = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => bytes
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => bytes
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        other => bail!("unsupported PNG colour type {other:?}"),
    };
    Ok((info.width, info.height, rgba))
}

/// PSNR in dB over RGB, and the fraction of pixels whose RGB differs by more than 2.
pub fn psnr(a: &[u8], b: &[u8]) -> (f64, f64) {
    let mut se = 0f64;
    let mut differing = 0usize;
    let n = a.len().min(b.len()) / 4;
    for i in 0..n {
        let mut d2 = 0f64;
        for k in 0..3 {
            let d = a[i * 4 + k] as f64 - b[i * 4 + k] as f64;
            d2 += d * d;
        }
        se += d2;
        if d2 > 12.0 {
            differing += 1;
        }
    }
    let mse = se / (n as f64 * 3.0);
    let psnr = if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    };
    (psnr, differing as f64 / n as f64)
}

pub fn run(a: &Path, b: &Path, min_psnr: Option<f64>) -> Result<()> {
    let (wa, ha, pa) = load_rgba(a)?;
    let (wb, hb, pb) = load_rgba(b)?;
    if (wa, ha) != (wb, hb) {
        bail!("sizes differ: {wa}x{ha} vs {wb}x{hb}");
    }
    let (p, frac) = psnr(&pa, &pb);
    println!("PSNR {p:.2} dB, {:.2}% of pixels differ", frac * 100.0);
    if let Some(min) = min_psnr {
        if p < min {
            bail!("PSNR {p:.2} below {min}");
        }
    }
    Ok(())
}
