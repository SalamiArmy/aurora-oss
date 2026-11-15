//! A tiny software 2D canvas: anti-aliased discs, rings, arcs, lines, convex polygons, additive glow and 8x8 text.
//! Pixels are `0x00RRGGBB`, which is what softbuffer expects.

use font8x8::legacy::BASIC_LEGACY;

pub type Rgb = [u8; 3];

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub buf: Vec<u32>,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Self {
        Canvas { w, h, buf: vec![0; (w.max(1) * h.max(1)) as usize] }
    }

    pub fn resize(&mut self, w: i32, h: i32) {
        if w != self.w || h != self.h {
            self.w = w;
            self.h = h;
            self.buf = vec![0; (w.max(1) * h.max(1)) as usize];
        }
    }

    /// Normal alpha blend of one pixel.
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let p = &mut self.buf[(y * self.w + x) as usize];
        let (r, g, b) = ((*p >> 16) & 255, (*p >> 8) & 255, *p & 255);
        let ia = 1.0 - a;
        let nr = (r as f32 * ia + c[0] as f32 * a) as u32;
        let ng = (g as f32 * ia + c[1] as f32 * a) as u32;
        let nb = (b as f32 * ia + c[2] as f32 * a) as u32;
        *p = (nr.min(255) << 16) | (ng.min(255) << 8) | nb.min(255);
    }

    /// Additive blend (glow).
    #[inline]
    pub fn add(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let p = &mut self.buf[(y * self.w + x) as usize];
        let (r, g, b) = ((*p >> 16) & 255, (*p >> 8) & 255, *p & 255);
        let nr = (r + (c[0] as f32 * a) as u32).min(255);
        let ng = (g + (c[1] as f32 * a) as u32).min(255);
        let nb = (b + (c[2] as f32 * a) as u32).min(255);
        *p = (nr << 16) | (ng << 8) | nb;
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgb, a: f32) {
        let (x0, y0) = (x.round() as i32, y.round() as i32);
        let (x1, y1) = ((x + w).round() as i32, (y + h).round() as i32);
        for yy in y0.max(0)..y1.min(self.h) {
            for xx in x0.max(0)..x1.min(self.w) {
                self.blend(xx, yy, c, a);
            }
        }
    }

    pub fn rect_outline(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgb, a: f32) {
        self.rect(x, y, w, 1.0, c, a);
        self.rect(x, y + h - 1.0, w, 1.0, c, a);
        self.rect(x, y, 1.0, h, c, a);
        self.rect(x + w - 1.0, y, 1.0, h, c, a);
    }

    /// Filled anti-aliased disc.
    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, a: f32) {
        let (x0, x1) = ((cx - r - 1.0).floor() as i32, (cx + r + 1.0).ceil() as i32);
        let (y0, y1) = ((cy - r - 1.0).floor() as i32, (cy + r + 1.0).ceil() as i32);
        for y in y0.max(0)..=y1.min(self.h - 1) {
            for x in x0.max(0)..=x1.min(self.w - 1) {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let cov = (r - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(x, y, c, cov * a);
                }
            }
        }
    }

    /// Anti-aliased ring of the given thickness centred on radius `r`.
    pub fn ring(&mut self, cx: f32, cy: f32, r: f32, thick: f32, c: Rgb, a: f32) {
        self.arc(cx, cy, r, thick, 0.0, std::f32::consts::TAU, c, a);
    }

    /// Ring segment from angle `a0` sweeping `sweep` radians (counter-clockwise on screen coordinates).
    pub fn arc(&mut self, cx: f32, cy: f32, r: f32, thick: f32, a0: f32, sweep: f32, c: Rgb, a: f32) {
        if sweep <= 0.0 {
            return;
        }
        let full = sweep >= std::f32::consts::TAU - 1e-3;
        let ext = r + thick;
        let (x0, x1) = ((cx - ext - 1.0).floor() as i32, (cx + ext + 1.0).ceil() as i32);
        let (y0, y1) = ((cy - ext - 1.0).floor() as i32, (cy + ext + 1.0).ceil() as i32);
        for y in y0.max(0)..=y1.min(self.h - 1) {
            for x in x0.max(0)..=x1.min(self.w - 1) {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = (thick * 0.5 - (d - r).abs() + 0.5).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                if !full {
                    let mut ang = dy.atan2(dx) - a0;
                    ang = ang.rem_euclid(std::f32::consts::TAU);
                    if ang > sweep {
                        continue;
                    }
                }
                self.blend(x, y, c, cov * a);
            }
        }
    }

    /// Additive radial glow: intensity falls off quadratically to zero at `r`.
    pub fn glow(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, strength: f32) {
        let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
        let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
        for y in y0.max(0)..=y1.min(self.h - 1) {
            for x in x0.max(0)..=x1.min(self.w - 1) {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt() / r;
                if d < 1.0 {
                    let k = (1.0 - d) * (1.0 - d) * strength;
                    self.add(x, y, c, k);
                }
            }
        }
    }

    /// Line by stamping small discs along it (cheap for the short and medium lines used here).
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, thick: f32, c: Rgb, a: f32) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt();
        // clip hopeless lines (both ends far off-screen on the same side)
        let m = 50.0;
        if (x0 < -m && x1 < -m) || (y0 < -m && y1 < -m) || (x0 > self.w as f32 + m && x1 > self.w as f32 + m) || (y0 > self.h as f32 + m && y1 > self.h as f32 + m) {
            return;
        }
        let step = (thick * 0.5).max(0.5);
        let n = ((len / step).ceil() as i32).clamp(1, 4000);
        let r = (thick * 0.5).max(0.6);
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let (px, py) = (x0 + dx * t, y0 + dy * t);
            if r <= 0.8 {
                // 1px lines: plain pixel plot is enough and far faster
                self.blend(px.floor() as i32, py.floor() as i32, c, a);
            } else {
                self.disc(px, py, r, c, a * 0.6);
            }
        }
    }

    pub fn dashed_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, dash: f32, c: Rgb, a: f32) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1.0 {
            return;
        }
        let n = (len / dash).floor() as i32;
        for i in (0..n).step_by(2) {
            let t0 = i as f32 * dash / len;
            let t1 = ((i + 1) as f32 * dash / len).min(1.0);
            self.line(x0 + dx * t0, y0 + dy * t0, x0 + dx * t1, y0 + dy * t1, 1.0, c, a);
        }
    }

    /// Fill a convex polygon (any winding) with alpha.
    pub fn convex(&mut self, pts: &[(f32, f32)], c: Rgb, a: f32) {
        if pts.len() < 3 {
            return;
        }
        let minx = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
        let maxx = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().min(self.w as f32 - 1.0) as i32;
        let miny = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
        let maxy = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().min(self.h as f32 - 1.0) as i32;
        // orientation
        let mut area = 0.0;
        for i in 0..pts.len() {
            let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
            area += p.0 * q.1 - q.0 * p.1;
        }
        let sign = if area >= 0.0 { 1.0 } else { -1.0 };
        for y in miny..=maxy {
            for x in minx..=maxx {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let mut inside = true;
                for i in 0..pts.len() {
                    let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                    let cross = (q.0 - p.0) * (py - p.1) - (q.1 - p.1) * (px - p.0);
                    if cross * sign < 0.0 {
                        inside = false;
                        break;
                    }
                }
                if inside {
                    self.blend(x, y, c, a);
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------ text
    pub fn text_width(s: &str, scale: i32) -> i32 {
        s.chars().count() as i32 * 8 * scale
    }

    pub fn text(&mut self, x: f32, y: f32, s: &str, scale: i32, c: Rgb, a: f32) {
        let mut px = x.round() as i32;
        let py = y.round() as i32;
        for ch in s.chars() {
            let idx = if (ch as usize) < 128 { ch as usize } else { b'?' as usize };
            let glyph = BASIC_LEGACY[idx];
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..8 {
                    if bits & (1 << col) != 0 {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                self.blend(px + col * scale + sx, py + row as i32 * scale + sy, c, a);
                            }
                        }
                    }
                }
            }
            px += 8 * scale;
        }
    }

    pub fn text_centered(&mut self, cx: f32, y: f32, s: &str, scale: i32, c: Rgb, a: f32) {
        let w = Self::text_width(s, scale) as f32;
        self.text(cx - w / 2.0, y, s, scale, c, a);
    }
}

pub fn scale_rgb(c: Rgb, k: f32) -> Rgb {
    [(c[0] as f32 * k).min(255.0) as u8, (c[1] as f32 * k).min(255.0) as u8, (c[2] as f32 * k).min(255.0) as u8]
}

pub fn mix_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t) as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}
