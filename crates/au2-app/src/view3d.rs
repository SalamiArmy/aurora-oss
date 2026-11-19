//! The perspective camera the board is seen through. The simulation stays 2D (the board is the plane z = 0, world
//! units, y up); only presentation is 3D. The camera looks at `target` from above, tilted towards the far (+y) edge.
//!
//! All of this is `f32` presentation maths; nothing here feeds back into the deterministic simulation except the
//! ground point under the cursor, which only decides which ships / targets the player picked.

/// Tilt of the view from straight down, in degrees.
pub const TILT_DEG: f32 = 48.0;
/// Vertical field of view, in degrees.
pub const FOV_DEG: f32 = 32.0;
/// Points nearer than this (along the view direction) are not drawn.
pub const NEAR: f32 = 50.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl V3 {
    pub const fn new(x: f32, y: f32, z: f32) -> V3 {
        V3 { x, y, z }
    }
    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: V3) -> V3 {
        V3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn norm(self) -> V3 {
        let l = self.len();
        if l < 1e-12 {
            self
        } else {
            self * (1.0 / l)
        }
    }
}
impl std::ops::Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl std::ops::Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl std::ops::Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, k: f32) -> V3 {
        V3::new(self.x * k, self.y * k, self.z * k)
    }
}
impl std::ops::Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        V3::new(-self.x, -self.y, -self.z)
    }
}

/// A projected point: screen position and distance along the view direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Proj {
    pub x: f32,
    pub y: f32,
    pub depth: f32,
    /// Screen pixels per world unit at this depth.
    pub scale: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: V3,
    pub right: V3,
    pub up: V3,
    pub fwd: V3,
    /// Focal length in pixels.
    pub f: f32,
    pub cx: f32,
    pub cy: f32,
}

impl Camera {
    /// A camera looking at ground point (`tx`, `ty`) so that the ground there is drawn at `zoom` pixels per world unit,
    /// centred at (`cx`, `cy`) on a view `view_h` pixels tall.
    pub fn new(tx: f32, ty: f32, zoom: f32, cx: f32, cy: f32, view_h: f32) -> Camera {
        let f = (view_h / 2.0) / (FOV_DEG.to_radians() / 2.0).tan();
        let dist = f / zoom.max(1e-6);
        let (s, c) = TILT_DEG.to_radians().sin_cos();
        let fwd = V3::new(0.0, s, -c);
        let right = V3::new(1.0, 0.0, 0.0);
        let up = V3::new(0.0, c, s);
        let target = V3::new(tx, ty, 0.0);
        Camera { pos: target - fwd * dist, right, up, fwd, f, cx, cy }
    }

    pub fn project(&self, p: V3) -> Option<Proj> {
        let rel = p - self.pos;
        let depth = rel.dot(self.fwd);
        if depth < NEAR {
            return None;
        }
        let k = self.f / depth;
        Some(Proj { x: self.cx + rel.dot(self.right) * k, y: self.cy - rel.dot(self.up) * k, depth, scale: k })
    }

    /// Depth of a world point along the view direction (negative = behind the camera).
    pub fn depth_of(&self, p: V3) -> f32 {
        (p - self.pos).dot(self.fwd)
    }

    /// The point on the plane z = `z` under screen point (`sx`, `sy`), if the ray hits it in front of the camera.
    pub fn unproject(&self, sx: f32, sy: f32, z: f32) -> Option<V3> {
        let dir = self.fwd + self.right * ((sx - self.cx) / self.f) + self.up * (-(sy - self.cy) / self.f);
        if dir.z.abs() < 1e-9 {
            return None;
        }
        let t = (z - self.pos.z) / dir.z;
        (t > 0.0).then(|| self.pos + dir * t)
    }

    /// Clip the segment a-b to the part in front of the near plane.
    pub fn clip(&self, a: V3, b: V3) -> Option<(V3, V3)> {
        let (da, db) = (self.depth_of(a) - NEAR * 1.01, self.depth_of(b) - NEAR * 1.01);
        if da < 0.0 && db < 0.0 {
            return None;
        }
        if da >= 0.0 && db >= 0.0 {
            return Some((a, b));
        }
        let t = da / (da - db);
        let m = a + (b - a) * t;
        Some(if da >= 0.0 { (a, m) } else { (m, b) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_is_at_the_centre_at_the_requested_scale() {
        let c = Camera::new(100.0, -50.0, 0.1, 640.0, 380.0, 760.0);
        let p = c.project(V3::new(100.0, -50.0, 0.0)).unwrap();
        assert!((p.x - 640.0).abs() < 1e-3 && (p.y - 380.0).abs() < 1e-3);
        assert!((p.scale - 0.1).abs() < 1e-6);
    }

    #[test]
    fn unproject_inverts_project_on_the_ground_and_far_things_look_smaller() {
        let c = Camera::new(0.0, 0.0, 0.08, 640.0, 380.0, 760.0);
        for (x, y) in [(0.0, 0.0), (3000.0, 2000.0), (-4000.0, -2500.0)] {
            let p = c.project(V3::new(x, y, 0.0)).unwrap();
            let w = c.unproject(p.x, p.y, 0.0).unwrap();
            assert!((w.x - x).abs() < 0.5 && (w.y - y).abs() < 0.5, "{w:?}");
        }
        let near = c.project(V3::new(0.0, -3000.0, 0.0)).unwrap();
        let far = c.project(V3::new(0.0, 3000.0, 0.0)).unwrap();
        assert!(near.scale > far.scale, "perspective");
        assert!(far.y < near.y, "+y is further up the screen");
        // a deeper layer moves less when the camera pans: parallax
        let c2 = Camera::new(1000.0, 0.0, 0.08, 640.0, 380.0, 760.0);
        let shift = |z: f32| c.project(V3::new(0.0, 0.0, z)).unwrap().x - c2.project(V3::new(0.0, 0.0, z)).unwrap().x;
        assert!(shift(0.0) > shift(-5000.0) && shift(-5000.0) > shift(-20000.0));
    }
}
