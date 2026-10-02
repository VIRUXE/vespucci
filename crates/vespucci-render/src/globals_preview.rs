//! Values for the engine-owned constant buffers that make the game's forward
//! techniques render sensibly without the real frame pipeline. Every entry
//! here exists because the `.fxc` default produced black or NaN output; see
//! `docs/binding.md`.

use crate::camera::{pack, Camera};
use crate::material::Globals;
use glam::{Mat4, Vec3};

pub fn apply(
    globals: &mut Globals,
    cam: &Camera,
    width: u32,
    height: u32,
    sun_sign: f32,
    transpose: bool,
) {
    let view = cam.view();
    let proj = cam.proj();
    set_world(globals, Mat4::IDENTITY, &view, &proj, transpose);
    if let Some(m) = globals.get_mut("misc_globals") {
        m.set_by_name(
            "globalScreenSize",
            &[
                width as f32,
                height as f32,
                1.0 / width as f32,
                1.0 / height as f32,
            ],
        );
        // The forward pixel shaders scale their final colour by globalScalars3.z (0 in the .fxc defaults).
        m.set_by_name("globalScalars3", &[16.0, 0.0625, 1.0, 1.0]);
    }
    let sun = Vec3::new(-0.4, -0.3, -0.85).normalize() * sun_sign;
    if let Some(m) = globals.get_mut("lighting_globals") {
        m.set_by_name("gDirectionalLight", &[sun.x, sun.y, sun.z, 0.0]);
        m.set_by_name("gDirectionalColour", &[1.0, 0.98, 0.92, 1.0]);
        for name in [
            "gLightNaturalAmbient0",
            "gLightNaturalAmbient1",
            "gLightArtificialIntAmbient0",
            "gLightArtificialIntAmbient1",
            "gLightArtificialExtAmbient0",
            "gLightArtificialExtAmbient1",
        ] {
            m.set_by_name(name, &[0.35, 0.37, 0.42, 1.0]);
        }
        m.set_by_name("gDirectionalAmbientColour", &[0.2, 0.2, 0.2, 1.0]);
        m.set_by_name("gNumForwardLights", &[0.0]);
        // Fog off: a start distance beyond everything, and non-zero exponents in the two
        // sun-scatter terms (regs 3/4 .w), whose `log(0) * 0` would otherwise be NaN.
        let mut fog = [0.0f32; 20];
        fog[0] = 1.0e8;
        fog[12] = sun.x;
        fog[13] = sun.y;
        fog[14] = sun.z;
        fog[15] = 1.0;
        fog[16] = sun.x;
        fog[17] = sun.y;
        fog[18] = sun.z;
        fog[19] = 1.0;
        m.set_by_name("globalFogParams", &fog);
    }
    if let Some(m) = globals.get_mut("more_stuff") {
        m.set_by_name("gAmbientOcclusionEffect", &[1.0, 1.0, 1.0, 1.0]);
        m.set_by_name("gDynamicBakesAndWetness", &[1.0, 1.0, 0.0, 0.0]);
        m.set_by_name("gReflectionMipCount", &[4.0]);
        // Alpha test for the `…CutOut` techniques: `PS_Textured_Zero_CutOut` discards when
        // gAlphaRefVec0.x >= alpha (the .fxc default of 0 discards nothing, so cutout LOD
        // billboards drew as solid quads). The game's value comes with the time cycle (#2).
        m.set_by_name("gAlphaRefVec0", &[0.5, 0.5, 0.5, 0.5]);
        m.set_by_name("gAlphaRefVec1", &[0.5, 0.5, 0.5, 0.5]);
    }
    // Cascaded shadows: a rotation with tiny cascade scales so every shadow test passes
    // against the 1x1 "depth 1.0" texture and the derivatives stay finite. The rotation
    // must have no zero entries: the foliage shaders divide by the Jacobian of the shadow
    // coordinates, and an axis-aligned mapping makes it exactly 0 on camera-facing quads.
    if let Some(m) = globals.get_mut("csmshader") {
        let mut v = [0.0f32; 48];
        let rot = glam::Mat3::from_axis_angle(Vec3::new(1.0, 1.0, 1.0).normalize(), 1.0);
        for r in 0..3 {
            let row = rot.row(r);
            v[r * 4..r * 4 + 3].copy_from_slice(&[row.x, row.y, row.z]);
        }
        for r in 4..8 {
            v[r * 4] = 1e-3;
            v[r * 4 + 1] = 1e-3;
            v[r * 4 + 2] = 1e-3;
        }
        m.set_by_name("gCSMShaderVars_shared", &v);
        m.set_by_name("gCSMResolution", &[1.0, 1.0, 1.0, 1.0]);
    }
    // Debug: VESPUCCI_SET_GLOBAL=cbuffer:var=a,b,c,d;cbuffer:var=... overrides preview globals.
    if let Ok(spec) = std::env::var("VESPUCCI_SET_GLOBAL") {
        for item in spec.split(';') {
            if let Some((target, values)) = item.split_once('=') {
                if let Some((cb, var)) = target.split_once(':') {
                    let v: Vec<f32> = values
                        .split(',')
                        .filter_map(|x| x.trim().parse().ok())
                        .collect();
                    let ok = globals.get_mut(cb).is_some_and(|m| m.set_by_name(var, &v));
                    log::info!(
                        "VESPUCCI_SET_GLOBAL {cb}:{var} = {v:?} -> {}",
                        if ok { "set" } else { "no such variable" }
                    );
                }
            }
        }
    }
}

/// Per-draw matrices for one instance.
/// Per-frame values that some shaders keep in their own cbuffer (`FrameParams`):
/// the cable shader's view-projection and its pixel-size parameters.
pub fn set_frame(
    globals: &mut Globals,
    cam: &Camera,
    view: &Mat4,
    proj: &Mat4,
    height: u32,
    transpose: bool,
) {
    globals
        .frame
        .view_proj
        .copy_from_slice(&pack(*proj * *view, transpose)[..]);
    let pixels_per_metre = 0.5 * height as f32 / (cam.fov_deg.to_radians() * 0.5).tan();
    globals.frame.cable_params = [pixels_per_metre, 1.0, 1.0, 1.0];
}

pub fn set_world(globals: &mut Globals, world: Mat4, view: &Mat4, proj: &Mat4, transpose: bool) {
    if let Some(m) = globals.get_mut("rage_matrices") {
        m.set_by_name("gWorld", &pack(world, transpose));
        m.set_by_name("gWorldView", &pack(*view * world, transpose));
        m.set_by_name("gWorldViewProj", &pack(*proj * *view * world, transpose));
        m.set_by_name("gViewInverse", &pack(view.inverse(), transpose));
    }
}
