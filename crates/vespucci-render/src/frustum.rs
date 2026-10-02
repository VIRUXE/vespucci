//! View-frustum test for bounding spheres.

use glam::{Mat4, Vec3, Vec4};

pub struct Frustum {
    /// Planes as (normal, d): inside when dot(n, p) + d >= 0.
    planes: [Vec4; 6],
}

impl Frustum {
    /// From a column-vector clip matrix (`proj * view`), D3D depth range 0..1.
    pub fn from_clip(m: Mat4) -> Frustum {
        let r = |i: usize| Vec4::new(m.x_axis[i], m.y_axis[i], m.z_axis[i], m.w_axis[i]);
        let (r0, r1, r2, r3) = (r(0), r(1), r(2), r(3));
        let planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2];
        Frustum { planes: planes.map(|p| p / p.truncate().length().max(1e-6)) }
    }

    pub fn contains_sphere(&self, centre: Vec3, radius: f32) -> bool {
        self.planes.iter().all(|p| p.truncate().dot(centre) + p.w + radius >= 0.0)
    }
}
