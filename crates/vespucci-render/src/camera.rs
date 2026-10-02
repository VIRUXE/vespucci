//! Camera and the matrices the game's shaders take. The world is Z-up.

use glam::{Mat4, Vec3};

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub position: Vec3,
    pub target: Vec3,
    pub fov_deg: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.position, self.target, Vec3::Z)
    }

    pub fn proj(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_deg.to_radians(), self.aspect, self.near, self.far)
    }

    /// Reversed-Z projection (near maps to 1, far to 0): with a float depth
    /// buffer this keeps coplanar decals and distant geometry from z-fighting.
    pub fn proj_reversed(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_deg.to_radians(), self.aspect, self.far, self.near)
    }

    /// Frames a bounding sphere: looks at its centre from the given angles.
    pub fn orbit(centre: Vec3, radius: f32, aspect: f32, yaw_deg: f32, pitch_deg: f32) -> Camera {
        let fov: f32 = 45.0;
        let dist = radius.max(0.01) / (fov.to_radians() * 0.5).sin() * 1.1;
        let (yaw, pitch) = (yaw_deg.to_radians(), pitch_deg.to_radians());
        let dir = Vec3::new(yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin());
        Camera { position: centre + dir * dist, target: centre, fov_deg: fov, aspect, near: (dist * 0.01).max(0.01), far: dist * 10.0 + radius }
    }
}

/// How a `float4x4` is packed for the shader. Verified on `normal_spec`'s
/// `unlit_draw`: glam's column-major `to_cols_array()` goes in as-is
/// (`transpose = false`); transposing puts the model off-screen.
pub fn pack(m: Mat4, transpose: bool) -> [f32; 16] {
    if transpose {
        m.transpose().to_cols_array()
    } else {
        m.to_cols_array()
    }
}
