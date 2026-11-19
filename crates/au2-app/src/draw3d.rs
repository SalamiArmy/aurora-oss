//! Software 3D on the canvas: ray-traced lit spheres (the suns), a flat-shaded triangle rasterizer for meshes (the
//! fragile-sun wreck uses `assets/rock.bin`), and a parallax starfield made of star layers below the board plane.
//! Every solid surface writes the canvas depth buffer so ships behind a sun are hidden by it.

use crate::gfx::{mix_rgb, Canvas, Rgb};
use crate::view3d::{Camera, V3};
use std::sync::OnceLock;

/// Key light direction (towards the light): from the upper left, in front of the board.
pub fn light_dir() -> V3 {
    V3::new(-0.45, -0.55, 0.70).norm()
}

// ---------------------------------------------------------------------------------------------- surface texture
const TEX_W: usize = 256;
const TEX_H: usize = 128;

fn hash2(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(0x8DA6_B343) ^ y.wrapping_mul(0xD816_3841) ^ seed.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x9E37_79B1);
    h ^ (h >> 16)
}

/// Tileable value noise, several octaves, values 0..1; wraps in x (longitude) so the sphere has no seam.
fn surface_texture() -> &'static Vec<f32> {
    static TEX: OnceLock<Vec<f32>> = OnceLock::new();
    TEX.get_or_init(|| {
        let mut t = vec![0.0f32; TEX_W * TEX_H];
        let mut amp = 1.0;
        let mut total = 0.0;
        for (oct, cells) in [8usize, 16, 32, 64].iter().enumerate() {
            let cy = (cells / 2).max(1);
            let val = |i: usize, j: usize| (hash2((i % cells) as u32, j.min(cy) as u32, oct as u32 + 7) & 0xffff) as f32 / 65535.0;
            for y in 0..TEX_H {
                for x in 0..TEX_W {
                    let fx = x as f32 / TEX_W as f32 * *cells as f32;
                    let fy = y as f32 / TEX_H as f32 * cy as f32;
                    let (i, j) = (fx as usize, fy as usize);
                    let (tx, ty) = (fx.fract(), fy.fract());
                    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
                    let a = val(i, j) * (1.0 - sx) + val(i + 1, j) * sx;
                    let b = val(i, j + 1) * (1.0 - sx) + val(i + 1, j + 1) * sx;
                    t[y * TEX_W + x] += (a * (1.0 - sy) + b * sy) * amp;
                }
            }
            total += amp;
            amp *= 0.5;
        }
        // swirled bands: a warped sine of latitude, like a stellar surface
        for y in 0..TEX_H {
            for x in 0..TEX_W {
                let n = t[y * TEX_W + x] / total;
                let band = 0.5 + 0.5 * (y as f32 / TEX_H as f32 * 18.0 + n * 7.0).sin();
                t[y * TEX_W + x] = (0.55 * n + 0.45 * band).clamp(0.0, 1.0);
            }
        }
        t
    })
}

fn sample(u: f32, v: f32) -> f32 {
    let t = surface_texture();
    let x = ((u.rem_euclid(1.0)) * TEX_W as f32) as usize % TEX_W;
    let y = ((v.clamp(0.0, 0.999)) * TEX_H as f32) as usize;
    t[y * TEX_W + x]
}

// ---------------------------------------------------------------------------------------------- spheres
/// How a sphere looks.
#[derive(Clone, Copy)]
pub struct SphereLook {
    pub color: Rgb,
    /// 0..1: how much it shines by itself (suns glow; neutral ones are dimmer).
    pub emissive: f32,
    /// Rotation about the board's vertical axis, radians (the surface turns).
    pub spin: f32,
    /// 0..1 strength of the swirling surface texture.
    pub texture: f32,
}

/// Draw a lit, textured sphere of world radius `r` centred at `c`, writing depth. Returns its screen centre and
/// radius (None if not visible).
pub fn sphere(cv: &mut Canvas, cam: &Camera, c: V3, r: f32, look: SphereLook) -> Option<(f32, f32, f32)> {
    let p = cam.project(c)?;
    let rad = r * p.scale;
    if rad < 0.5 {
        return None;
    }
    let ext = rad * 1.15 + 2.0;
    let (x0, x1) = (((p.x - ext).floor() as i32).max(0), ((p.x + ext).ceil() as i32).min(cv.w - 1));
    let (y0, y1) = (((p.y - ext).floor() as i32).max(0), ((p.y + ext).ceil() as i32).min(cv.h - 1));
    if x0 > x1 || y0 > y1 {
        return Some((p.x, p.y, rad));
    }
    let l = light_dir();
    let oc = cam.pos - c;
    let cc = oc.dot(oc) - r * r;
    let (sn, cs) = look.spin.sin_cos();
    let white: Rgb = [255, 250, 240];
    for y in y0..=y1 {
        for x in x0..=x1 {
            // the ray through this pixel, hit against the sphere (exact under perspective)
            let d = (cam.fwd + cam.right * ((x as f32 + 0.5 - cam.cx) / cam.f) + cam.up * (-(y as f32 + 0.5 - cam.cy) / cam.f)).norm();
            let b = oc.dot(d);
            let disc = b * b - cc;
            if disc <= 0.0 {
                continue;
            }
            let sq = disc.sqrt();
            let t = -b - sq;
            if t <= 0.0 {
                continue;
            }
            let hit = cam.pos + d * t;
            let depth = (hit - cam.pos).dot(cam.fwd);
            if !cv.visible(x, y, depth) {
                continue;
            }
            let n = (hit - c) * (1.0 / r);
            // surface texture in the sphere's own (spinning) frame
            let (nx, ny) = (n.x * cs - n.y * sn, n.x * sn + n.y * cs);
            let u = ny.atan2(nx) / std::f32::consts::TAU + 0.5;
            let v = n.z * 0.5 + 0.5;
            let tex = sample(u, v);
            let lam = n.dot(l).max(0.0);
            let view = -d;
            let ndv = n.dot(view).max(0.0);
            let rim = (1.0 - ndv).powi(3);
            let h = (l + view).norm();
            let spec = n.dot(h).max(0.0).powi(28) * 0.45;
            let shade = look.emissive + (1.0 - look.emissive) * (0.18 + 0.82 * lam);
            let surf = 1.0 - look.texture * 0.55 + look.texture * 0.75 * tex;
            let k = (shade * surf).min(1.6);
            let mut col = [
                (look.color[0] as f32 * k).min(255.0) as u8,
                (look.color[1] as f32 * k).min(255.0) as u8,
                (look.color[2] as f32 * k).min(255.0) as u8,
            ];
            col = mix_rgb(col, mix_rgb(look.color, white, 0.6), (rim * 0.9 + spec).min(1.0));
            // anti-aliased silhouette
            let edge = (sq / (r * 0.04 + 1e-3)).min(1.0);
            cv.put_depth(x, y, depth, col, edge.max(0.55));
        }
    }
    Some((p.x, p.y, rad))
}

// ---------------------------------------------------------------------------------------------- meshes
pub struct Mesh {
    pub verts: Vec<V3>,
    pub faces: Vec<[u16; 3]>,
}

/// The faceted rock from `assets/rock.bin` (generated by `tools/gen_oss_assets.py`): u16 vertex and face counts,
/// then f32 xyz per vertex, then u16 triples. Compiled in, so every platform has it.
pub fn rock() -> &'static Mesh {
    static ROCK: OnceLock<Mesh> = OnceLock::new();
    ROCK.get_or_init(|| parse_mesh(include_bytes!("../../../assets/rock.bin")).unwrap_or(Mesh { verts: Vec::new(), faces: Vec::new() }))
}

pub fn parse_mesh(b: &[u8]) -> Option<Mesh> {
    let u16_at = |i: usize| -> Option<u16> { Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?])) };
    let f32_at = |i: usize| -> Option<f32> { Some(f32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?)) };
    let (nv, nf) = (u16_at(0)? as usize, u16_at(2)? as usize);
    let mut verts = Vec::with_capacity(nv);
    for k in 0..nv {
        let o = 4 + k * 12;
        verts.push(V3::new(f32_at(o)?, f32_at(o + 4)?, f32_at(o + 8)?));
    }
    let base = 4 + nv * 12;
    let mut faces = Vec::with_capacity(nf);
    for k in 0..nf {
        let o = base + k * 6;
        let f = [u16_at(o)?, u16_at(o + 2)?, u16_at(o + 4)?];
        if f.iter().any(|&i| i as usize >= nv) {
            return None;
        }
        faces.push(f);
    }
    Some(Mesh { verts, faces })
}

fn rotate_z(p: V3, a: f32) -> V3 {
    let (s, c) = a.sin_cos();
    V3::new(p.x * c - p.y * s, p.x * s + p.y * c, p.z)
}
fn rotate_x(p: V3, a: f32) -> V3 {
    let (s, c) = a.sin_cos();
    V3::new(p.x, p.y * c - p.z * s, p.y * s + p.z * c)
}

/// Draw `mesh` scaled by `size`, rotated (`rx` about x, then `rz` about z) and placed at `at`, flat-shaded.
pub fn draw_mesh(cv: &mut Canvas, cam: &Camera, mesh: &Mesh, at: V3, size: f32, rx: f32, rz: f32, color: Rgb) {
    let l = light_dir();
    let world: Vec<V3> = mesh.verts.iter().map(|&v| rotate_z(rotate_x(v, rx), rz) * size + at).collect();
    for f in &mesh.faces {
        let (a, b, c) = (world[f[0] as usize], world[f[1] as usize], world[f[2] as usize]);
        let n = (b - a).cross(c - a).norm();
        let centre = (a + b + c) * (1.0 / 3.0);
        if n.dot(cam.pos - centre) <= 0.0 {
            continue; // back face
        }
        let (Some(pa), Some(pb), Some(pc)) = (cam.project(a), cam.project(b), cam.project(c)) else { continue };
        let k = 0.25 + 0.75 * n.dot(l).max(0.0);
        let col = [(color[0] as f32 * k) as u8, (color[1] as f32 * k) as u8, (color[2] as f32 * k) as u8];
        triangle(cv, [(pa.x, pa.y, pa.depth), (pb.x, pb.y, pb.depth), (pc.x, pc.y, pc.depth)], col);
    }
}

/// Depth-tested filled triangle (screen x, y, depth per corner).
pub fn triangle(cv: &mut Canvas, p: [(f32, f32, f32); 3], col: Rgb) {
    let minx = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let maxx = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil().min(cv.w as f32 - 1.0) as i32;
    let miny = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let maxy = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil().min(cv.h as f32 - 1.0) as i32;
    let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
    if area.abs() < 1e-6 {
        return;
    }
    for y in miny..=maxy {
        for x in minx..=maxx {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1].0 - px) * (p[2].1 - py) - (p[2].0 - px) * (p[1].1 - py)) / area;
            let w1 = ((p[2].0 - px) * (p[0].1 - py) - (p[0].0 - px) * (p[2].1 - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let depth = w0 * p[0].2 + w1 * p[1].2 + w2 * p[2].2;
            cv.put_depth(x, y, depth, col, 1.0);
        }
    }
}

// ---------------------------------------------------------------------------------------------- starfield
/// Star layers below the board plane: (depth below the board, base cell size, density 0..1, brightness).
/// Deeper layers move less as the camera pans and zooms: parallax.
const LAYERS: [(f32, f32, u32, f32); 4] = [(1500.0, 700.0, 35, 1.0), (5000.0, 900.0, 45, 0.85), (14000.0, 1400.0, 55, 0.7), (40000.0, 2600.0, 70, 0.55)];
const MAX_CELLS: f32 = 2600.0;

/// Draw the parallax starfield (and a few faint nebulae on the deepest layer). `t` = wall-clock seconds (twinkle).
pub fn starfield(cv: &mut Canvas, cam: &Camera, t: f32) {
    let (w, h) = (cv.w as f32, cv.h as f32);
    for (li, &(depth, base, density, bright)) in LAYERS.iter().enumerate() {
        let z = -depth;
        // the part of this layer's plane that is on screen
        let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];
        let pts: Vec<V3> = corners.iter().filter_map(|&(x, y)| cam.unproject(x, y, z)).collect();
        if pts.len() < 4 {
            continue;
        }
        let (x0, x1) = (pts.iter().map(|p| p.x).fold(f32::MAX, f32::min), pts.iter().map(|p| p.x).fold(f32::MIN, f32::max));
        let (y0, y1) = (pts.iter().map(|p| p.y).fold(f32::MAX, f32::min), pts.iter().map(|p| p.y).fold(f32::MIN, f32::max));
        // coarser lattices when zoomed out; each coarser level keeps a subset of the finer one's stars (no popping
        // of the stars that stay)
        let mut step = 1i64;
        while ((x1 - x0) / (base * step as f32)) * ((y1 - y0) / (base * step as f32)) > MAX_CELLS {
            step *= 2;
        }
        let (i0, i1) = (((x0 / base).floor() as i64).div_euclid(step) * step, (x1 / base).ceil() as i64);
        let (j0, j1) = (((y0 / base).floor() as i64).div_euclid(step) * step, (y1 / base).ceil() as i64);
        let mut i = i0;
        while i <= i1 {
            let mut j = j0;
            while j <= j1 {
                let hsh = hash2(i as u32, j as u32, li as u32 * 977 + 3);
                if hsh % 100 < density {
                    let fx = ((hsh >> 8) & 0xff) as f32 / 255.0;
                    let fy = ((hsh >> 16) & 0xff) as f32 / 255.0;
                    let pos = V3::new((i as f32 + fx) * base, (j as f32 + fy) * base, z);
                    if let Some(p) = cam.project(pos) {
                        let kind = hsh >> 24;
                        let tint: Rgb = match kind % 7 {
                            0 => [255, 214, 170],
                            1 => [170, 200, 255],
                            2 => [255, 180, 160],
                            _ => [235, 240, 255],
                        };
                        let twinkle = 0.8 + 0.2 * (t * (1.0 + (kind % 5) as f32) + kind as f32).sin();
                        let a = bright * (0.25 + 0.75 * ((kind & 63) as f32 / 63.0)) * twinkle;
                        let (sx, sy) = (p.x.floor() as i32, p.y.floor() as i32);
                        cv.add(sx, sy, tint, a);
                        if kind % 11 == 0 && li < 2 {
                            // a few brighter, bigger stars in the near layers
                            cv.add(sx + 1, sy, tint, a * 0.5);
                            cv.add(sx - 1, sy, tint, a * 0.5);
                            cv.add(sx, sy + 1, tint, a * 0.5);
                            cv.add(sx, sy - 1, tint, a * 0.5);
                        }
                    }
                }
                j += step;
            }
            i += step;
        }
        // nebulae: large faint haze on the deepest layer
        if li == LAYERS.len() - 1 {
            let nb = 60_000.0;
            for i in ((x0 / nb).floor() as i64)..=((x1 / nb).ceil() as i64) {
                for j in ((y0 / nb).floor() as i64)..=((y1 / nb).ceil() as i64) {
                    let hsh = hash2(i as u32, j as u32, 4242);
                    let pos = V3::new((i as f32 + ((hsh & 0xff) as f32 / 255.0)) * nb, (j as f32 + (((hsh >> 8) & 0xff) as f32 / 255.0)) * nb, z);
                    if let Some(p) = cam.project(pos) {
                        let col: Rgb = match hsh % 3 {
                            0 => [70, 40, 120],
                            1 => [30, 70, 120],
                            _ => [110, 40, 70],
                        };
                        let r = (25_000.0 * p.scale).clamp(40.0, 260.0);
                        cv.glow(p.x, p.y, r, col, 0.35);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rock_mesh_asset_parses() {
        let m = rock();
        assert_eq!((m.verts.len(), m.faces.len()), (42, 80));
        assert!(parse_mesh(&[1, 0, 1, 0]).is_none(), "truncated data is rejected");
    }

    #[test]
    fn a_sphere_hides_what_is_behind_it() {
        let mut cv = Canvas::new(400, 300);
        let cam = Camera::new(0.0, 0.0, 0.2, 200.0, 150.0, 300.0);
        let look = SphereLook { color: [200, 120, 60], emissive: 0.5, spin: 0.0, texture: 0.5 };
        let (x, y, r) = sphere(&mut cv, &cam, V3::new(0.0, 0.0, 0.0), 100.0, look).unwrap();
        assert!((r - 20.0).abs() < 2.0, "{r}");
        // a ship on the board just behind the sun (further +y) is hidden; one in front is visible
        let behind = cam.project(V3::new(0.0, 60.0, 0.0)).unwrap();
        let front = cam.project(V3::new(0.0, -140.0, 0.0)).unwrap();
        assert!(!cv.visible(behind.x as i32, behind.y as i32, behind.depth));
        assert!(cv.visible(front.x as i32, front.y as i32, front.depth));
        assert!(cv.buf[(y as i32 * 400 + x as i32) as usize] != 0, "drawn");
    }

    #[test]
    fn a_mesh_is_rasterized_with_depth() {
        let mut cv = Canvas::new(200, 200);
        let cam = Camera::new(0.0, 0.0, 0.5, 100.0, 100.0, 200.0);
        draw_mesh(&mut cv, &cam, rock(), V3::new(0.0, 0.0, 0.0), 60.0, 0.3, 0.7, [200, 200, 200]);
        let drawn = cv.depth.iter().filter(|&&d| d < f32::MAX).count();
        assert!(drawn > 200, "{drawn}");
    }

    #[test]
    fn starfield_draws_and_shifts_less_for_deeper_layers() {
        let mut cv = Canvas::new(320, 200);
        let cam = Camera::new(0.0, 0.0, 0.05, 160.0, 100.0, 200.0);
        starfield(&mut cv, &cam, 0.0);
        assert!(cv.buf.iter().filter(|&&p| p != 0).count() > 50);
    }
}
