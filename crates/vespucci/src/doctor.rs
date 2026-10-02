//! `vespucci doctor`: environment checks, and with `--gpu` the M0 spike — a
//! triangle drawn through DXVK-native on lavapipe, read back and written as PNG.

use anyhow::{bail, Result};
use std::path::Path;
use vespucci_d3d11::ffi::*;
use vespucci_d3d11::{Device, InputElement};

const TRI_VS: &[u8] = include_bytes!("../../../shaders/m0/tri.vs.dxbc");
const TRI_PS: &[u8] = include_bytes!("../../../shaders/m0/tri.ps.dxbc");

const CLEAR: [f32; 4] = [0.2, 0.4, 0.6, 1.0];

struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

fn check(name: &'static str, ok: bool, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok,
        detail: detail.into(),
    }
}

pub fn run(game: Option<&Path>, gpu: bool, out: Option<&Path>, size: u32) -> Result<()> {
    let mut checks = Vec::new();

    match game {
        Some(dir) => {
            for f in ["GTA5.exe", "common.rpf", "x64a.rpf", "update/update.rpf"] {
                let p = dir.join(f);
                checks.push(check("game file", p.exists(), p.display().to_string()));
            }
            let version = vespucci_game::keys::resolve_exe(dir)
                .ok()
                .and_then(|exe| vespucci_game::keys::exe_version(&exe));
            checks.push(check(
                "game build",
                version.is_some(),
                version
                    .clone()
                    .unwrap_or_else(|| "version resource not found".into()),
            ));
            match vespucci_game::keys::load(dir) {
                Ok((_, cached)) => checks.push(check(
                    "RPF keys",
                    true,
                    if cached {
                        "from cache"
                    } else {
                        "recovered from GTA5.exe"
                    },
                )),
                Err(e) => checks.push(check("RPF keys", false, e.to_string())),
            }
            let t = std::time::Instant::now();
            match vespucci_game::GameFs::open(dir) {
                Ok(fs) => checks.push(check(
                    "archives",
                    fs.archives.len() >= 100 && fs.files.len() >= 100_000,
                    format!(
                        "{} archives, {} files, scanned in {:.1} s",
                        fs.archives.len(),
                        fs.files.len(),
                        t.elapsed().as_secs_f64()
                    ),
                )),
                Err(e) => checks.push(check("archives", false, e.to_string())),
            }
        }
        None => checks.push(check("game dir", false, "not given (--game or $GTAV_PATH)")),
    }

    // Headless Linux needs lavapipe + DXVK-native; on Windows D3D11 is native.
    #[cfg(not(windows))]
    {
        let icd = "/usr/share/vulkan/icd.d/lvp_icd.json";
        checks.push(check("lavapipe ICD", Path::new(icd).exists(), icd));
        for lib in [
            "/opt/dxvk-native/lib/x86_64-linux-gnu/libdxvk_d3d11.so",
            "/opt/dxvk-native/lib/x86_64-linux-gnu/libdxvk_dxgi.so",
        ] {
            checks.push(check("DXVK-native lib", Path::new(lib).exists(), lib));
        }
    }

    for c in &checks {
        println!(
            "{} {:<16} {}",
            if c.ok { "PASS" } else { "FAIL" },
            c.name,
            c.detail
        );
    }
    let failed = checks.iter().filter(|c| !c.ok).count();

    if gpu {
        println!("GPU test:");
        gpu_test(out, size)?;
    }

    if failed > 0 {
        bail!("{failed} check(s) failed");
    }
    Ok(())
}

fn gpu_test(out: Option<&Path>, size: u32) -> Result<()> {
    let dev = crate::timed("create device", Device::create)?;
    let name = dev.adapter_description()?;
    println!("  adapter: {name} (feature level {:#x})", dev.feature_level);
    if !name.to_ascii_lowercase().contains("llvmpipe") {
        println!("  note: not running on lavapipe");
    }

    let rt = dev.create_render_target(size, size, DXGI_FORMAT_R8G8B8A8_UNORM)?;
    let vs = dev.create_vertex_shader(TRI_VS)?;
    let ps = dev.create_pixel_shader(TRI_PS)?;
    let layout = dev.create_input_layout(
        &[
            InputElement::new("POSITION", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 0),
            InputElement::new("COLOR", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 12),
        ],
        TRI_VS,
    )?;
    // x, y, z, r, g, b, a — a red triangle covering the middle of the target.
    let verts: [f32; 21] = [
        0.0, 0.7, 0.0, 1.0, 0.0, 0.0, 1.0, //
        0.7, -0.7, 0.0, 1.0, 0.0, 0.0, 1.0, //
        -0.7, -0.7, 0.0, 1.0, 0.0, 0.0, 1.0,
    ];
    let vbytes: &[u8] = unsafe {
        core::slice::from_raw_parts(verts.as_ptr() as *const u8, core::mem::size_of_val(&verts))
    };
    let vb = dev.create_buffer(vbytes, D3D11_BIND_VERTEX_BUFFER, D3D11_USAGE_IMMUTABLE)?;

    let draw = || -> Result<Vec<u8>> {
        let ctx = dev.ctx.as_ptr();
        dev.clear(&rt, CLEAR);
        dev.bind_render_target(&rt);
        let bufs = [vb.as_ptr()];
        let strides = [28u32];
        let offsets = [0u32];
        unsafe {
            vespucci_d3d11::com_call!(ctx, IASetInputLayout, layout.as_ptr());
            vespucci_d3d11::com_call!(
                ctx,
                IASetVertexBuffers,
                0,
                1,
                bufs.as_ptr(),
                strides.as_ptr(),
                offsets.as_ptr()
            );
            vespucci_d3d11::com_call!(
                ctx,
                IASetPrimitiveTopology,
                D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST
            );
            vespucci_d3d11::com_call!(ctx, VSSetShader, vs.as_ptr(), core::ptr::null(), 0);
            vespucci_d3d11::com_call!(ctx, PSSetShader, ps.as_ptr(), core::ptr::null(), 0);
            vespucci_d3d11::com_call!(ctx, Draw, 3, 0);
        }
        dev.read_back(&rt.texture, 4)
    };

    let first = crate::timed("first draw + readback (pipeline JIT)", draw)?;
    let second = crate::timed("second draw + readback", draw)?;
    if first != second {
        bail!("frames differ between runs");
    }

    let px = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [first[i], first[i + 1], first[i + 2], first[i + 3]]
    };
    let corner = px(4, 4);
    let centre = px(size / 2, size / 2);
    let expect_clear = [51u8, 102, 153, 255];
    println!(
        "  corner pixel {:?} (expect ~{:?}), centre pixel {:?} (expect red)",
        corner, expect_clear, centre
    );
    let clear_ok = corner
        .iter()
        .zip(expect_clear)
        .all(|(a, b)| (*a as i32 - b as i32).abs() <= 1);
    let red_ok = centre == [255, 0, 0, 255];

    if let Some(p) = out {
        crate::write_png(p, size, size, &first)?;
        println!("  wrote {}", p.display());
    }
    if !clear_ok || !red_ok {
        bail!("pixel check failed");
    }
    println!("  GPU test PASS");
    Ok(())
}
