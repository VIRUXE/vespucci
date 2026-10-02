//! From the linear HDR frame the game's shaders produce to display pixels.
//! The real game runs exposure, bloom and a filmic curve here; this is the
//! plain preview version (fixed exposure, ACES fit, gamma 2.2).

pub fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let mant = (h & 0x3ff) as f32;
    match exp {
        0 => sign * mant * 2f32.powi(-24),
        31 => if mant == 0.0 { sign * f32::INFINITY } else { f32::NAN },
        _ => sign * (1.0 + mant / 1024.0) * 2f32.powi(exp - 15),
    }
}

fn aces(x: f32) -> f32 {
    ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
}

/// RGBA16F rows to RGBA8 with the display transform. NaN/Inf become magenta so
/// shader-input mistakes stay visible; the count of such pixels is returned.
pub fn rgba16f_to_rgba8(pixels: &[u8], exposure: f32) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(pixels.len() / 2);
    let mut bad = 0;
    for px in pixels.chunks_exact(8) {
        let c: Vec<f32> = (0..3).map(|i| half_to_f32(u16::from_le_bytes([px[i * 2], px[i * 2 + 1]]))).collect();
        if c.iter().any(|v| !v.is_finite()) {
            out.extend_from_slice(&[255, 0, 255, 255]);
            bad += 1;
            continue;
        }
        for v in c {
            let t = aces(v * exposure).powf(1.0 / 2.2);
            out.push((t * 255.0 + 0.5) as u8);
        }
        out.push(255);
    }
    (out, bad)
}
