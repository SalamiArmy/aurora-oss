//! Drawing the game and its HUD onto the software canvas.

use crate::draw3d;
use crate::game::{lasso_circle, Game, Input, Overlay};
use crate::view3d::{Camera, V3};
use crate::gfx::{mix_rgb, Canvas, Rgb};
use au2_map::TerrainKind;
use au2_sim::math::Vec2;
use au2_sim::rules::{sun_radius, CAPTURE_COST, GAMMA_HALF_LENGTH, MAX_HEALTH, NOVA_COLLAPSE_TICKS, TICKS_PER_SEC, UPGRADE_COST};
use au2_sim::ShockShape;
use au2_sim::{FragilePhase, Mode, NovaPhase, Obj, Status, WhitePhase};

pub const FACTION_COLORS: [Rgb; 4] = [[80, 200, 255], [255, 120, 70], [110, 230, 110], [235, 110, 235]];
pub const NEUTRAL: Rgb = [150, 150, 165];
pub const WHITE: Rgb = [235, 240, 255];
const TAU: f32 = std::f32::consts::TAU;
const TOP: f32 = -std::f32::consts::FRAC_PI_2;
/// Suns, white holes and supernovas are drawn this much larger than their simulated radius, so they read as
/// solid bodies at the fitted zoom (the circling clouds start at 2.4 radii, so they still clear them).
const BODY_SCALE: f32 = 1.4;

pub fn faction_color(f: i8) -> Rgb {
    if f < 0 {
        NEUTRAL
    } else {
        FACTION_COLORS[(f as usize).min(3)]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiAction {
    Resume,
    Restart,
    Surrender,
    Levels,
    Next,
}

/// Immediate-mode button; returns true when clicked this frame.
pub fn button(c: &mut Canvas, input: &Input, x: f32, y: f32, w: f32, h: f32, label: &str, active: bool) -> bool {
    let hover = input.mouse.0 >= x && input.mouse.0 < x + w && input.mouse.1 >= y && input.mouse.1 < y + h;
    let base: Rgb = if active { [40, 90, 140] } else { [26, 34, 56] };
    c.rect(x, y, w, h, if hover { mix_rgb(base, WHITE, 0.15) } else { base }, 0.95);
    c.rect_outline(x, y, w, h, if active { [120, 200, 255] } else { [70, 85, 120] }, 1.0);
    let scale = if h >= 34.0 && w >= Canvas::text_width(label, 2) as f32 + 12.0 { 2 } else { 1 };
    c.text_centered(x + w / 2.0, y + (h - 8.0 * scale as f32) / 2.0, label, scale, WHITE, 1.0);
    hover && input.left_pressed
}

fn hash(n: u32) -> u32 {
    let mut x = n.wrapping_mul(0x9E37_79B1);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^ (x >> 13)
}

pub fn background(c: &mut Canvas) {
    background_gradient(c);
    // fixed star field in screen space (menus; the game draws a 3D parallax starfield instead)
    for i in 0..140u32 {
        let x = (hash(i * 2) % c.w.max(1) as u32) as i32;
        let y = (hash(i * 2 + 1) % c.h.max(1) as u32) as i32;
        let a = 0.15 + (hash(i + 999) % 100) as f32 / 400.0;
        c.blend(x, y, WHITE, a);
    }
}

fn background_gradient(c: &mut Canvas) {
    let h = c.h as f32;
    for y in 0..c.h {
        let t = y as f32 / h;
        let col = [(4.0 + 6.0 * t) as u8, (5.0 + 6.0 * t) as u8, (12.0 + 12.0 * t) as u8];
        let p = ((col[0] as u32) << 16) | ((col[1] as u32) << 8) | col[2] as u32;
        let row = (y * c.w) as usize;
        c.buf[row..row + c.w as usize].fill(p);
    }
}

// ---------------------------------------------------------------------------------------------- ground-plane helpers
fn g3(p: Vec2) -> V3 {
    V3::new(p.x as f32, p.y as f32, 0.0)
}

/// Projected ground points of a circle (None if any is behind the camera).
fn ground_circle_pts(cam: &Camera, centre: Vec2, r: f64, n: usize) -> Option<Vec<(f32, f32)>> {
    (0..n)
        .map(|k| {
            let a = k as f64 / n as f64 * std::f64::consts::TAU;
            cam.project(V3::new((centre.x + r * a.cos()) as f32, (centre.y + r * a.sin()) as f32, 0.0)).map(|p| (p.x, p.y))
        })
        .collect()
}

fn segments(n_px: f32) -> usize {
    (n_px / 6.0).clamp(24.0, 180.0) as usize
}

/// A filled disc lying on the board (an ellipse on screen).
fn ground_disc(c: &mut Canvas, cam: &Camera, centre: Vec2, r: f64, col: Rgb, a: f32) {
    let Some(p) = cam.project(g3(centre)) else { return };
    if let Some(pts) = ground_circle_pts(cam, centre, r, segments(r as f32 * p.scale * 6.28)) {
        c.convex(&pts, col, a);
    }
}

/// An arc lying on the board, from angle `a0` sweeping `sweep` radians (world angles, counter-clockwise), drawn with a
/// depth test so the parts behind a sun are hidden.
fn ground_arc(c: &mut Canvas, cam: &Camera, centre: Vec2, r: f64, a0: f64, sweep: f64, thick: f32, col: Rgb, a: f32) {
    let Some(p) = cam.project(g3(centre)) else { return };
    let px = (r as f32 * p.scale * sweep.abs() as f32).max(1.0);
    let n = (px / 0.8).clamp(8.0, 3000.0) as usize;
    let rad = (thick * 0.5).max(0.5);
    for k in 0..=n {
        let ang = a0 + sweep * k as f64 / n as f64;
        let w = V3::new((centre.x + r * ang.cos()) as f32, (centre.y + r * ang.sin()) as f32, 0.0);
        let Some(q) = cam.project(w) else { continue };
        if q.x < -10.0 || q.y < -10.0 || q.x > c.w as f32 + 10.0 || q.y > c.h as f32 + 10.0 || !c.visible(q.x as i32, q.y as i32, q.depth) {
            continue;
        }
        if rad <= 0.8 {
            c.blend(q.x as i32, q.y as i32, col, a);
        } else {
            c.disc(q.x, q.y, rad, col, a * 0.5);
        }
    }
}

fn ground_ring(c: &mut Canvas, cam: &Camera, centre: Vec2, r: f64, thick: f32, col: Rgb, a: f32) {
    ground_arc(c, cam, centre, r, 0.0, std::f64::consts::TAU, thick, col, a);
}

/// A straight line on the board, clipped to what is in front of the camera.
fn ground_line(c: &mut Canvas, cam: &Camera, a: Vec2, b: Vec2, thick: f32, col: Rgb, al: f32, dashed: bool) {
    let Some((a, b)) = cam.clip(g3(a), g3(b)) else { return };
    let (Some(p), Some(q)) = (cam.project(a), cam.project(b)) else { return };
    if dashed {
        c.dashed_line(p.x, p.y, q.x, q.y, 12.0, col, al);
    } else {
        c.line(p.x, p.y, q.x, q.y, thick, col, al);
    }
}

/// A band on the board between two points, `half` wide either side (clipped), filled.
fn ground_band(c: &mut Canvas, cam: &Camera, a: Vec2, b: Vec2, half: f64, col: Rgb, al: f32) {
    let Some((a, b)) = cam.clip(g3(a), g3(b)) else { return };
    let d = b - a;
    let n = V3::new(-d.y, d.x, 0.0).norm() * half as f32;
    let pts: Option<Vec<(f32, f32)>> = [a + n, b + n, b - n, a - n].iter().map(|&p| cam.project(p).map(|q| (q.x, q.y))).collect();
    if let Some(pts) = pts {
        c.convex(&pts, col, al);
    }
}

pub fn draw_game(c: &mut Canvas, g: &Game, input: &Input) {
    let view = (c.w as f32, c.h as f32);
    let cam = g.camera(view);
    let t = g.clock;
    let sim = &g.sim;
    let tick = sim.tick;
    let vision = sim.vision(0);
    let sp = |p: Vec2| g.to_screen(p, view);
    let k = |p: Vec2| cam.project(g3(p)).map(|q| q.scale).unwrap_or(0.0);
    let on_screen = |x: f32, y: f32, m: f32| x > -m && y > -m && x < view.0 + m && y < view.1 + m;

    c.clear_depth();
    background_gradient(c);
    draw3d::starfield(c, &cam, t);

    // ------------------------------------------------------------------ terrain (on the board plane)
    for o in &sim.objs {
        let Obj::Terrain(tr) = o else { continue };
        if !tr.enabled {
            continue;
        }
        let d = &tr.def;
        let col: Rgb = match d.kind {
            TerrainKind::Force => [90, 140, 255],
            TerrainKind::Speed => {
                if d.speed_change >= 0.0 {
                    [80, 255, 190]
                } else {
                    [255, 190, 80]
                }
            }
            TerrainKind::Reflect => [225, 230, 255],
            TerrainKind::Minefield => [255, 70, 70],
            TerrainKind::Confusion => [190, 90, 255],
            TerrainKind::Neutral => [190, 190, 190],
        };
        if d.outer_radius > 0.0 {
            let centre = Vec2::new(d.pos[0], d.pos[1]);
            ground_disc(c, &cam, centre, d.outer_radius, col, 0.10);
            ground_ring(c, &cam, centre, d.outer_radius, 1.5, col, 0.6);
            if d.inner_radius > 0.0 {
                ground_ring(c, &cam, centre, d.inner_radius, 1.5, col, 0.6);
            }
            if d.kind == TerrainKind::Force {
                for q in 0..12 {
                    let at = centre + au2_sim::math::dir_deg(q as f64 * 30.0) * ((d.inner_radius + d.outer_radius) / 2.0);
                    force_arrow(c, &cam, at, tr.push_at(at), (d.outer_radius - d.inner_radius) * 0.4, t, q, col);
                }
            }
            continue;
        }
        for r in 0..d.rows {
            for cc in 0..d.cols {
                if d.data.get((r * d.cols + cc) as usize).copied().unwrap_or(0) == 0 {
                    continue;
                }
                let (wx, wy, ts) = (d.pos[0] + cc as f64 * d.tile_size, d.pos[1] + r as f64 * d.tile_size, d.tile_size);
                let corners = [Vec2::new(wx, wy), Vec2::new(wx + ts, wy), Vec2::new(wx + ts, wy + ts), Vec2::new(wx, wy + ts)];
                let pts: Option<Vec<(f32, f32)>> = corners.iter().map(|&p| cam.project(g3(p)).map(|q| (q.x, q.y))).collect();
                let Some(pts) = pts else { continue };
                if pts.iter().all(|p| !on_screen(p.0, p.1, 0.0)) && !pts.iter().any(|p| p.0 >= 0.0 && p.0 <= view.0) {
                    continue;
                }
                let mid = Vec2::new(wx + ts / 2.0, wy + ts / 2.0);
                let alpha = match d.kind {
                    TerrainKind::Reflect => 0.55,
                    TerrainKind::Minefield => 0.04 + 0.12 * tr.mines_left(mid) as f32,
                    _ => 0.13,
                };
                c.convex(&pts, col, alpha);
                match d.kind {
                    TerrainKind::Force => force_arrow(c, &cam, mid, tr.push_at(mid), ts, t, r + cc, col),
                    TerrainKind::Minefield => {
                        // fewer blinking mines where ships have cleared them
                        let left = (tr.mines_left(mid) * 4.0).ceil() as u32;
                        for q in 0..left {
                            let h = hash(r * 131 + cc * 17 + q);
                            let (fx, fy) = ((h & 255) as f64 / 255.0, ((h >> 8) & 255) as f64 / 255.0);
                            let (x, y) = sp(Vec2::new(wx + fx * ts, wy + fy * ts));
                            let blink = 0.5 + 0.5 * (t * 3.0 + h as f32).sin();
                            c.disc(x, y, 1.5, col, 0.4 + 0.5 * blink);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ------------------------------------------------------------------ gamma lanes and shockwaves
    for o in &sim.objs {
        match o {
            Obj::Gamma(gm) => {
                let hw = gm.def.width / 2.0;
                let (a, b) = (gm.pos - gm.dir * GAMMA_HALF_LENGTH, gm.pos + gm.dir * GAMMA_HALF_LENGTH);
                let n = gm.dir.perp();
                let col: Rgb = if gm.def.kill_units { [255, 60, 60] } else { [255, 170, 60] };
                let warn = gm.warning(tick);
                let flash = if warn { 0.12 + 0.12 * (t * 14.0).sin().abs() } else { 0.06 };
                ground_band(c, &cam, a, b, hw, col, flash);
                for s in [1.0, -1.0] {
                    ground_line(c, &cam, a + n * (hw * s), b + n * (hw * s), 1.0, col, if warn { 0.8 } else { 0.3 }, true);
                }
                let (ox, oy) = sp(gm.pos);
                c.glow(ox, oy, 600.0 * k(gm.pos) + 10.0, col, 0.5);
                if let Some(front) = gm.front(tick) {
                    // the pulse: a bright slab sweeping along the lane
                    let f = gm.pos + gm.dir * front;
                    let back = gm.pos + gm.dir * (front - 2500.0).max(-GAMMA_HALF_LENGTH);
                    ground_band(c, &cam, back, f, hw, [255, 240, 200], 0.55);
                    let (fx, fy) = sp(f);
                    c.glow(fx, fy, (hw as f32 * 2.0) * k(f) + 10.0, [255, 230, 160], 0.9);
                }
            }
            Obj::Shock(sh) => {
                let thick = (au2_sim::rules::SHOCK_THICKNESS as f32 * k(sh.pos)).max(2.0);
                match sh.wall(tick) {
                    Some(ShockShape::Ring(r)) => ground_ring(c, &cam, sh.pos, r, thick, [255, 230, 140], 0.35),
                    Some(ShockShape::Line { normal, offset }) => {
                        let mid = sh.pos + normal * offset;
                        let along = normal.perp();
                        ground_band(c, &cam, mid - along * 30000.0, mid + along * 30000.0, au2_sim::rules::SHOCK_THICKNESS / 2.0, [255, 230, 140], 0.3);
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ orbit paths
    for o in &sim.objs {
        if let Obj::Sun(s) = o {
            if let Some(or) = &s.orbit {
                let rot = or.rotation_deg.to_radians();
                let mut prev: Option<(f32, f32)> = None;
                for i in 0..=96 {
                    let a = i as f64 / 96.0 * std::f64::consts::TAU;
                    let (ex, ey) = (or.rx * a.cos(), or.ry * a.sin());
                    let w = Vec2::new(s.centre.x + ex * rot.cos() - ey * rot.sin(), s.centre.y + ex * rot.sin() + ey * rot.cos());
                    let p = cam.project(g3(w)).map(|q| (q.x, q.y));
                    if let (Some(q), Some(p)) = (prev, p) {
                        if i % 2 == 0 {
                            c.line(q.0, q.1, p.0, p.1, 1.0, NEUTRAL, 0.2);
                        }
                    }
                    prev = p;
                }
            }
        }
    }

    // ------------------------------------------------------------------ glows behind the solid objects
    for (_, s) in sim.suns() {
        if !s.present() || !(vision.visible(s.pos) || s.faction == 0) {
            continue;
        }
        let (x, y) = sp(s.pos);
        let r = s.radius as f32 * BODY_SCALE * k(s.pos);
        if on_screen(x, y, r * 4.0) {
            let owned = s.faction >= 0;
            c.glow(x, y, r * 3.2, faction_color(s.faction), if owned { 0.30 + 0.06 * s.level as f32 } else { 0.08 });
        }
    }

    // ------------------------------------------------------------------ wormholes, white holes, supernovas
    for (i, o) in sim.objs.iter().enumerate() {
        match o {
            Obj::Wormhole(w) => {
                let col: Rgb = [150, 120, 255];
                let (x, y) = sp(w.pos);
                c.glow(x, y, w.size as f32 * k(w.pos) * 2.0, col, 0.35);
                ground_disc(c, &cam, w.pos, w.size, [10, 6, 30], 0.85);
                for q in 0..3 {
                    let rr = w.size * (0.9 - 0.25 * q as f64);
                    let dir = if q % 2 == 0 { 1.0 } else { -1.0 };
                    ground_arc(c, &cam, w.pos, rr, (t * dir * (1.0 + q as f32 * 0.5) + q as f32) as f64, 3.6, 2.0, col, 0.8);
                }
                ground_ring(c, &cam, w.pos, w.size, 1.5, col, 0.8);
                if w.dest > i as i32 {
                    if let Some(p) = sim.obj_pos(w.dest as usize) {
                        ground_line(c, &cam, w.pos, p, 1.0, col, 0.18, true);
                    }
                }
            }
            Obj::WhiteHole(w) => {
                let a = if w.accepting() { 1.0 } else { 0.45 };
                let look = draw3d::SphereLook { color: [235, 240, 255], emissive: 0.75 * a + 0.2, spin: t * 0.3, texture: 0.35 };
                let Some((x, y, r)) = draw3d::sphere(c, &cam, g3(w.pos), w.radius() as f32 * 0.7, look) else { continue };
                c.glow(x, y, r * 3.0, WHITE, 0.4 * a);
                if let WhitePhase::Firing { dir, faction, .. } = w.phase {
                    let col = faction_color(faction as i8);
                    for q in -2..=2 {
                        let d = dir.rotate(q as f64 * 15.0);
                        ground_line(c, &cam, w.pos, w.pos + d * (w.radius() * 4.0), 2.0, col, 0.35, false);
                    }
                }
                if w.accepting() {
                    if w.investor >= 0 {
                        c.arc(x, y, r * 1.4 + 3.0, 3.0, TOP, w.invest as f32 / w.required() as f32 * TAU, faction_color(w.investor), 0.9);
                    }
                    c.text_centered(x, y + r * 1.5 + 6.0, &format!("{} ships", w.prize(tick)), 1, WHITE, 0.8);
                }
            }
            Obj::Supernova(s) => {
                let col: Rgb = [255, 170, 60];
                let base = s.radius() as f32 * 0.6;
                let (r3, emissive) = match s.phase {
                    NovaPhase::Ready => (base * (1.0 + 0.06 * (t * 4.0).sin()), 0.8),
                    NovaPhase::Collapse { until, .. } => {
                        let q = 1.0 - (until.saturating_sub(tick)) as f32 / NOVA_COLLAPSE_TICKS as f32;
                        (base * (1.0 - 0.7 * q), 1.0)
                    }
                    _ => (base * 0.35, 0.3),
                };
                let look = draw3d::SphereLook { color: [255, 200, 120], emissive, spin: t * 0.6, texture: 0.6 };
                if let Some((x, y, r)) = draw3d::sphere(c, &cam, g3(s.pos), r3, look) {
                    c.glow(x, y, r * 3.0, col, 0.45);
                    if s.phase == NovaPhase::Ready && s.investor >= 0 {
                        c.arc(x, y, r * 1.4 + 3.0, 3.0, TOP, s.invest as f32 / s.required() as f32 * TAU, faction_color(s.investor), 0.9);
                    }
                }
                if let NovaPhase::Collapse { .. } = s.phase {
                    for q in 0..3 {
                        let ph = (t * 1.5 + q as f32 / 3.0).fract() as f64;
                        ground_ring(c, &cam, s.pos, s.def.collapse_radius * (1.0 - ph), 1.5, col, 0.35);
                    }
                }
                if let Some(front) = s.front(tick) {
                    ground_ring(c, &cam, s.pos, front, 6.0, [255, 230, 170], 0.85);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ suns: lit 3D spheres
    let mut labels = Vec::new();
    for (i, s) in sim.suns() {
        let (x, y) = sp(s.pos);
        let ks = k(s.pos);
        let r = s.radius as f32 * ks;
        let outer = sun_radius(s.max_level) as f32 * ks;
        if ks <= 0.0 || !on_screen(x, y, outer * 4.0 + 10.0) {
            continue;
        }
        let seen = vision.visible(s.pos) || s.faction == 0;
        if let Some(fr) = &s.fragile {
            match fr.phase {
                FragilePhase::Broken { .. } => {
                    // a small wreck: a few tumbling rocks (the generated rock mesh)
                    for q in 0..4u32 {
                        let h = hash(i as u32 * 31 + q);
                        let a = (h % 628) as f32 / 100.0;
                        let d = s.radius as f32 * 0.5 * ((h >> 10) % 100) as f32 / 100.0;
                        let at = V3::new(s.pos.x as f32 + a.cos() * d, s.pos.y as f32 + a.sin() * d, 0.0);
                        let size = s.radius as f32 * (0.18 + 0.12 * ((h >> 4) % 10) as f32 / 10.0);
                        draw3d::draw_mesh(c, &cam, draw3d::rock(), at, size, t * 0.4 + q as f32, t * 0.25 + a, [130, 118, 128]);
                    }
                    continue;
                }
                FragilePhase::Rebuilding { from, until } => {
                    let q = (tick - from) as f32 / (until - from).max(1) as f32;
                    let look = draw3d::SphereLook { color: NEUTRAL, emissive: 0.15, spin: t * 0.2, texture: 0.4 };
                    draw3d::sphere(c, &cam, g3(s.pos), s.radius as f32 * BODY_SCALE * q.max(0.05), look);
                    continue;
                }
                FragilePhase::Intact => {}
            }
        }
        if !seen {
            ground_ring(c, &cam, s.pos, s.radius, 1.5, NEUTRAL, 0.35);
            continue;
        }
        let col = faction_color(s.faction);
        let owned = s.faction >= 0;
        // pulse when a ring of ships leaves
        let since_ring = tick.saturating_sub(s.last_ring) as f32;
        let pulse = if owned && s.last_ring > 0 && since_ring < 12.0 { 1.0 + 0.10 * (1.0 - since_ring / 12.0) } else { 1.0 };
        let look = draw3d::SphereLook {
            color: if owned { mix_rgb(col, WHITE, 0.15) } else { [150, 150, 160] },
            emissive: if owned { 0.45 + 0.07 * s.level as f32 } else { 0.12 },
            spin: t * (0.12 + 0.03 * (i % 5) as f32) + i as f32,
            texture: if owned { 0.65 } else { 0.45 },
        };
        let Some((sx, sy, rr)) = draw3d::sphere(c, &cam, g3(s.pos), s.radius as f32 * pulse * BODY_SCALE, look) else { continue };
        // faint rings on the board marking the level steps still to come, up to the maximum
        for l in (s.level.max(1) + 1)..=s.max_level {
            ground_ring(c, &cam, s.pos, sun_radius(l) * BODY_SCALE as f64, 1.0, col, 0.3);
        }
        if s.fragile.is_some() {
            c.line(sx - rr * 0.5, sy - rr * 0.3, sx + rr * 0.2, sy + rr * 0.45, 1.0, WHITE, 0.35);
            c.line(sx + rr * 0.1, sy - rr * 0.6, sx - rr * 0.1, sy + rr * 0.1, 1.0, WHITE, 0.35);
        }
        labels.push((i, sx, sy, rr.max(r)));
    }

    // ------------------------------------------------------------------ ships (depth-tested against the suns)
    let sel_col = mix_rgb(FACTION_COLORS[0], WHITE, 0.75);
    for s in &sim.ships {
        if s.faction != 0 && !vision.visible(s.pos) {
            continue;
        }
        let Some(p) = cam.project(g3(s.pos)) else { continue };
        if !on_screen(p.x, p.y, 20.0) {
            continue;
        }
        let (xi, yi) = (p.x.floor() as i32, p.y.floor() as i32);
        if !c.visible(xi, yi, p.depth) {
            continue; // behind a sun
        }
        let selected = s.faction == 0 && !g.selection.is_empty() && g.is_selected(s.id);
        let col = if selected { sel_col } else { faction_color(s.faction as i8) };
        let tl = if s.mode == Mode::Fly { 30.0 } else { 12.0 };
        if let Some(q) = cam.project(g3(s.pos - s.heading * tl)) {
            c.line(p.x, p.y, q.x, q.y, 1.0, col, 0.4);
        }
        // nearer ships are a touch bigger
        let size = (p.scale * 9.0).clamp(0.0, 1.5);
        if s.strength > 1 {
            let r = 1.2 + (s.strength as f32).sqrt() * 0.7 + size;
            c.disc(p.x, p.y, r, col, 0.9);
            c.disc(p.x, p.y, r * 0.5, mix_rgb(col, WHITE, 0.6), 0.9);
        } else if size > 1.0 {
            c.disc(p.x, p.y, 1.0 + size * 0.5, mix_rgb(col, WHITE, 0.3), 0.95);
        } else {
            let b = mix_rgb(col, WHITE, 0.5);
            c.add(xi, yi, b, 1.0);
            c.add(xi + 1, yi, col, 0.9);
            c.add(xi, yi + 1, col, 0.9);
            c.add(xi + 1, yi + 1, col, 0.7);
        }
    }

    // ------------------------------------------------------------------ sun overlays (on top of their ships)
    for (i, x, y, rr) in labels {
        let s = sim.sun(i).unwrap();
        let col = faction_color(s.faction);
        if s.faction >= 0 {
            // damage: the missing share of health as a dark arc over the sun
            let maxh = MAX_HEALTH[s.level as usize].max(1);
            if s.health < maxh {
                let lost = 1.0 - s.health as f32 / maxh as f32;
                c.arc(x, y, rr * 0.75, (rr * 0.3).max(2.0), TOP, lost * TAU, [90, 10, 10], 0.6);
            }
            // two bars under the sun: health (green) and growth (white)
            let bw = (rr * 1.6).max(18.0);
            let by = y + rr + 6.0;
            c.rect(x - bw / 2.0, by, bw, 3.0, [30, 30, 40], 0.8);
            c.rect(x - bw / 2.0, by, bw * s.health as f32 / maxh as f32, 3.0, [120, 235, 140], 0.95);
            if s.level < s.max_level {
                let need = UPGRADE_COST[s.level as usize].max(1);
                c.rect(x - bw / 2.0, by + 5.0, bw, 3.0, [30, 30, 40], 0.8);
                c.rect(x - bw / 2.0, by + 5.0, bw * s.growth as f32 / need as f32, 3.0, WHITE, 0.95);
            }
        } else if s.cap_progress > 0 {
            let frac = s.cap_progress as f32 / CAPTURE_COST as f32;
            c.arc(x, y, rr + 5.0, 3.0, TOP, frac.clamp(0.0, 1.0) * TAU, faction_color(s.cap_faction), 0.9);
        }
        // level pips
        let n = s.max_level as i32;
        for q in 0..n {
            let px = x + (q as f32 - (n - 1) as f32 / 2.0) * 7.0;
            let py = y - rr - 10.0;
            if (q as u8) < s.level {
                c.disc(px, py, 2.5, col, 1.0);
            } else {
                c.ring(px, py, 2.5, 1.0, col, 0.5);
            }
        }
    }

    // ------------------------------------------------------------------ effects
    for sp_ in &g.sparks {
        let (x, y) = sp(sp_.pos);
        let q = sp_.age / 0.5;
        c.ring(x, y, 2.0 + q * 8.0, 1.2, faction_color(sp_.color_faction as i8), (1.0 - q) * 0.9);
        c.add(x as i32, y as i32, WHITE, 1.0 - q);
    }
    for p in &g.pings {
        let q = p.age / 0.8;
        let kk = k(p.pos).max(1e-4);
        ground_ring(c, &cam, p.pos, ((6.0 + q * 30.0) / kk) as f64, 2.0, WHITE, 1.0 - q);
    }
    for b in &g.bursts {
        // a dying sun: rays and debris in the owner's colour (big for supergiants)
        let (x, y) = sp(b.pos);
        let q = b.age / 1.6;
        let col = faction_color(b.faction);
        let reach = if b.big { 900.0 } else { 300.0 } * k(b.pos) * (0.3 + q);
        let rays = if b.big { 16 } else { 8 };
        for ray in 0..rays {
            let a = ray as f32 / rays as f32 * TAU + (hash(ray) % 100) as f32 / 300.0;
            let len = reach * (0.6 + (hash(ray + 77) % 100) as f32 / 250.0);
            c.line(x, y, x + a.cos() * len, y + a.sin() * len, if b.big { 2.0 } else { 1.0 }, mix_rgb(col, WHITE, 0.4), (1.0 - q) * 0.8);
            c.disc(x + a.cos() * len * 1.1, y + a.sin() * len * 1.1, 2.0, col, 1.0 - q);
        }
        c.glow(x, y, reach * 0.6, col, (1.0 - q) * 0.8);
    }

    // ------------------------------------------------------------------ selection feedback
    let modal = g.overlay != Overlay::None || sim.status != Status::Playing;
    if !modal && !g.selection.is_empty() && !input.touch_mode && input.mouse.1 < view.1 - g.bar_h && g.drag.is_none() {
        let wp = g.to_world(input.mouse, view);
        let hover = sim.pick(wp, (6.0 / k(wp).max(1e-4)) as f64);
        let (tx, ty, col) = match hover {
            Some(h) => {
                let pos = sim.obj_pos(h).unwrap();
                let col = match sim.objs.get(h) {
                    Some(Obj::Sun(s)) if s.faction == 0 => FACTION_COLORS[0],
                    Some(Obj::Sun(s)) if s.faction > 0 => [255, 90, 90],
                    Some(Obj::Sun(_)) => [255, 230, 120],
                    _ => WHITE,
                };
                let (x, y) = sp(pos);
                c.ring(x, y, 24.0, 2.0, col, 0.9);
                (x, y, col)
            }
            None => (input.mouse.0, input.mouse.1, if vision.visible(wp) { WHITE } else { [255, 90, 90] }),
        };
        // from the middle of the selection to the target
        let (mut cx, mut cy, mut n) = (0.0, 0.0, 0.0);
        for s in sim.ships.iter().filter(|s| s.faction == 0 && g.is_selected(s.id)) {
            cx += s.pos.x;
            cy += s.pos.y;
            n += 1.0;
        }
        if n > 0.0 {
            let (x, y) = sp(Vec2::new(cx / n, cy / n));
            c.dashed_line(x, y, tx, ty, 8.0, col, 0.55);
        }
    }
    if g.lasso.len() >= 2 {
        if let Some((cc, r)) = lasso_circle(&g.lasso) {
            ground_disc(c, &cam, cc, r, FACTION_COLORS[0], 0.08);
            ground_ring(c, &cam, cc, r, 2.0, FACTION_COLORS[0], 0.8);
        }
    }

    fog_overlay(c, g, &vision, view);
}

/// A small animated arrow on the board showing a force zone's push at `at`.
fn force_arrow(c: &mut Canvas, cam: &Camera, at: Vec2, push: Vec2, size: f64, t: f32, seed: u32, col: Rgb) {
    let l = push.len();
    if l < 1e-9 {
        return;
    }
    let u = push * (1.0 / l);
    let ph = ((t * 0.8 + seed as f32 * 0.3).fract() - 0.5) as f64;
    let mid = at + u * (ph * size * 0.6);
    let (a, b) = (mid - u * (size * 0.2), mid + u * (size * 0.2));
    if let (Some(p), Some(q)) = (cam.project(g3(a)), cam.project(g3(b))) {
        c.line(p.x, p.y, q.x, q.y, 1.5, col, 0.6);
        c.disc(q.x, q.y, 2.0, col, 0.8);
    }
}

/// Darken everything the human cannot see (levels with fog of war). Visible discs on the board are ellipses on screen.
fn fog_overlay(c: &mut Canvas, g: &Game, vision: &au2_sim::Vision, view: (f32, f32)) {
    if vision.all {
        return;
    }
    const CELL: i32 = 4;
    let (cw, ch) = (c.w / CELL + 2, c.h / CELL + 2);
    let mut mask = vec![0.0f32; (cw * ch) as usize];
    for (centre, radius) in &vision.circles {
        let (sx, sy) = g.to_screen(*centre, view);
        let rx = *radius as f32 * g.scale_at(*centre, view);
        let up = g.to_screen(*centre + Vec2::new(0.0, *radius), view);
        let down = g.to_screen(*centre - Vec2::new(0.0, *radius), view);
        let (top, bottom) = (up.1.min(sy), down.1.max(sy));
        if rx <= 0.0 || sx < -1.0e4 {
            continue;
        }
        let (cx, cr) = (sx / CELL as f32, rx / CELL as f32);
        let (ry_up, ry_down) = (((sy - top) / CELL as f32).max(0.5), ((bottom - sy) / CELL as f32).max(0.5));
        let cy = sy / CELL as f32;
        let (x0, x1) = (((cx - cr).floor() as i32 - 1).max(0), ((cx + cr).ceil() as i32 + 1).min(cw - 1));
        let (y0, y1) = (((cy - ry_up).floor() as i32 - 1).max(0), ((cy + ry_down).ceil() as i32 + 1).min(ch - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dy = y as f32 - cy;
                let ry = if dy < 0.0 { ry_up } else { ry_down };
                let d = ((x as f32 - cx) / cr.max(0.001)).powi(2) + (dy / ry).powi(2);
                let v = ((1.0 - d.sqrt()) / 0.2).clamp(0.0, 1.0);
                let m = &mut mask[(y * cw + x) as usize];
                if v > *m {
                    *m = v;
                }
            }
        }
    }
    for y in 0..c.h {
        let fy = y as f32 / CELL as f32;
        let y0 = fy.floor() as i32;
        let ty = fy - y0 as f32;
        for x in 0..c.w {
            let fx = x as f32 / CELL as f32;
            let x0 = fx.floor() as i32;
            let tx = fx - x0 as f32;
            let at = |xx: i32, yy: i32| mask[(yy.min(ch - 1) * cw + xx.min(cw - 1)) as usize];
            let m = (at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx) * (1.0 - ty) + (at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx) * ty;
            let k = 0.30 + 0.70 * m;
            if k < 0.999 {
                let p = &mut c.buf[(y * c.w + x) as usize];
                let (r, gg, b) = (((*p >> 16) & 255) as f32 * k, ((*p >> 8) & 255) as f32 * k, (*p & 255) as f32 * k);
                *p = ((r as u32) << 16) | ((gg as u32) << 8) | b as u32;
            }
        }
    }
}

/// HUD, bottom bar, overlays. Returns an action for the app to carry out.
pub fn draw_ui(c: &mut Canvas, g: &mut Game, input: &Input) -> Option<UiAction> {
    let view = (c.w as f32, c.h as f32);
    let sim = &g.sim;

    // ------------------------------------------------------------------ top-left info
    c.rect(0.0, 0.0, if input.touch_mode { 360.0 } else { 330.0 }, 74.0, [8, 10, 22], 0.6);
    c.text(10.0, 8.0, &format!("{} - {}", g.entry.constellation, g.entry.name), 2, WHITE, 1.0);
    let secs = g.seconds() as u32;
    c.text(10.0, 30.0, &format!("time {:02}:{:02}   AI level {}   {}{}", secs / 60, secs % 60, g.entry.difficulty, g.speed.label(), if g.paused { "  PAUSED" } else { "" }), 1, [180, 190, 220], 1.0);
    let sel: u32 = if g.selection.is_empty() { 0 } else { sim.ships.iter().filter(|s| s.faction == 0 && g.is_selected(s.id)).map(|s| s.strength).sum() };
    c.text(10.0, 44.0, &format!("selected {sel} ships"), 1, [180, 190, 220], 1.0);
    let hint = if input.touch_mode {
        if g.selection.is_empty() { "draw a circle or tap a sun to select ships" } else { "tap a target to send them" }
    } else if g.selection.is_empty() {
        "drag a circle / click a sun: select   F1: help"
    } else {
        "click a target: send   Esc: deselect   F1: help"
    };
    c.text(10.0, 58.0, hint, 1, [120, 130, 160], 1.0);

    // ------------------------------------------------------------------ top-right faction table
    let n = sim.faction_count();
    let x0 = view.0 - 190.0;
    c.rect(x0 - 8.0, 0.0, 198.0, 14.0 + 16.0 * n as f32, [8, 10, 22], 0.6);
    let vis = sim.vision(0);
    for f in 0..n {
        let col = faction_color(f as i8);
        let y = 8.0 + 16.0 * f as f32;
        c.disc(x0, y + 4.0, 4.0, col, if sim.factions[f].alive { 1.0 } else { 0.25 });
        let label = if f == 0 || vis.all {
            format!("{}: {} suns {} ships", if f == 0 { "you" } else { "AI" }, sim.sun_count(f as u8), sim.strength(f as u8))
        } else {
            "AI: ? suns ? ships".to_string()
        };
        c.text(x0 + 10.0, y, &label, 1, if sim.factions[f].alive { WHITE } else { [110, 110, 120] }, 1.0);
    }

    // ------------------------------------------------------------------ toast
    if let Some((msg, _)) = &g.toast {
        let w = Canvas::text_width(msg, 2) as f32 + 24.0;
        c.rect(view.0 / 2.0 - w / 2.0, view.1 - g.bar_h - 44.0, w, 30.0, [10, 14, 30], 0.85);
        c.text_centered(view.0 / 2.0, view.1 - g.bar_h - 36.0, msg, 2, WHITE, 1.0);
    }

    // ------------------------------------------------------------------ bottom bar
    let by = view.1 - g.bar_h;
    c.rect(0.0, by, view.0, g.bar_h, [12, 16, 30], 0.96);
    c.rect(0.0, by, view.0, 1.0, [70, 85, 120], 1.0);
    let (bh, y) = (g.bar_h - 14.0, by + 7.0);
    // (label, base width, active, gap before). Buttons shrink uniformly when the screen is too narrow (phone portrait).
    let pause = if g.paused { "Play" } else { "Pause" };
    let speed = g.speed.label();
    let items: [(&str, f32, bool, f32); 5] = [(pause, 70.0, g.paused, 0.0), (speed, 80.0, g.speed != crate::game::Speed::Normal, 4.0), ("Fit", 50.0, false, 14.0), ("All", 50.0, false, 4.0), ("None", 58.0, false, 4.0)];
    let menu_w = 80.0;
    let avail = view.0 - 16.0 - menu_w - 8.0;
    let need: f32 = items.iter().map(|i| i.1 + i.3).sum();
    let f = (avail / need).min(1.0);
    let mut x = 8.0;
    for (n, (label, w, active, gap)) in items.iter().enumerate() {
        x += gap * f;
        if button(c, input, x, y, w * f, bh, label, *active) {
            match n {
                0 => g.paused = !g.paused,
                1 => g.speed = g.speed.next(),
                2 => g.fit(view),
                3 => g.select_all(),
                _ => g.selection.clear(),
            }
        }
        x += w * f;
    }
    if button(c, input, view.0 - menu_w - 8.0, y, menu_w, bh, "Menu", false) {
        g.overlay = Overlay::Pause;
    }

    // ------------------------------------------------------------------ overlays
    if g.sim.status != Status::Playing {
        return results(c, g, input);
    }
    let mut action = None;
    if g.overlay == Overlay::Pause {
        c.rect(0.0, 0.0, view.0, view.1, [0, 0, 8], 0.55);
        let (pw, bh) = (360.0_f32.min(view.0 - 10.0), if input.touch_mode { 44.0 } else { 34.0 });
        let pitch = bh + 10.0;
        let ph = 100.0 + 4.0 * pitch;
        let (px, py) = (view.0 / 2.0 - pw / 2.0, (view.1 / 2.0 - ph / 2.0).max(0.0));
        c.rect(px, py, pw, ph, [14, 18, 36], 0.97);
        c.rect_outline(px, py, pw, ph, [90, 110, 160], 1.0);
        c.text_centered(view.0 / 2.0, py + 20.0, "PAUSED", 4, WHITE, 1.0);
        let (bx, bw) = (px + 40.0, pw - 80.0);
        let mut by = py + 80.0;
        for (label, a) in [("Resume", UiAction::Resume), ("Restart level", UiAction::Restart), ("Surrender", UiAction::Surrender), ("Level select", UiAction::Levels)] {
            if button(c, input, bx, by, bw, bh, label, false) {
                action = Some(a);
            }
            by += pitch;
        }
    }
    action
}

/// The results screen: outcome, game time, per-faction graphs over time, retry / next / exit.
fn results(c: &mut Canvas, g: &Game, input: &Input) -> Option<UiAction> {
    let view = (c.w as f32, c.h as f32);
    let sim = &g.sim;
    let won = sim.status == Status::Won;
    c.rect(0.0, 0.0, view.0, view.1, [0, 0, 8], 0.6);
    let pw = (view.0 - 20.0).min(760.0);
    let bh = if input.touch_mode { 44.0 } else { 34.0 };
    let graph_h = ((view.1 - 230.0) / 1.0).clamp(80.0, 170.0);
    let ph = 120.0 + graph_h + 30.0 + bh + 20.0;
    let (px, py) = (view.0 / 2.0 - pw / 2.0, (view.1 / 2.0 - ph / 2.0).max(0.0));
    c.rect(px, py, pw, ph, [14, 18, 36], 0.97);
    c.rect_outline(px, py, pw, ph, [90, 110, 160], 1.0);
    let (title, tcol): (&str, Rgb) = if won { ("VICTORY", [120, 255, 150]) } else { ("DEFEAT", [255, 110, 100]) };
    c.text_centered(view.0 / 2.0, py + 16.0, title, 4, tcol, 1.0);
    let secs = sim.tick / TICKS_PER_SEC as u64;
    c.text_centered(view.0 / 2.0, py + 56.0, &format!("{} - {}    game time {:02}:{:02}", g.entry.constellation, g.entry.name, secs / 60, secs % 60), 1, [180, 190, 220], 1.0);
    // legend
    let n = sim.faction_count();
    for f in 0..n {
        let lx = px + 20.0 + f as f32 * 90.0;
        c.disc(lx, py + 80.0, 4.0, faction_color(f as i8), 1.0);
        c.text(lx + 8.0, py + 76.0, if f == 0 { "you" } else { "AI" }, 1, WHITE, 1.0);
    }
    // three graphs side by side: ships, production, combat losses
    let gw = (pw - 40.0 - 2.0 * 16.0) / 3.0;
    let gy = py + 100.0;
    let series: [(&str, fn(&au2_sim::Sample, usize) -> u32); 3] = [("ships", |s, f| s.ships[f]), ("production /s", |s, f| s.production[f]), ("combat losses", |s, f| s.losses[f])];
    for (k, (name, get)) in series.iter().enumerate() {
        let gx = px + 20.0 + k as f32 * (gw + 16.0);
        c.rect(gx, gy, gw, graph_h, [8, 10, 22], 0.9);
        c.rect_outline(gx, gy, gw, graph_h, [60, 70, 100], 1.0);
        c.text(gx + 4.0, gy + 4.0, name, 1, [180, 190, 220], 1.0);
        let max = sim.stats.iter().flat_map(|s| (0..n).map(move |f| get(s, f))).max().unwrap_or(1).max(1) as f32;
        c.text(gx + gw - 4.0 - Canvas::text_width(&max.to_string(), 1) as f32, gy + 4.0, &max.to_string(), 1, [120, 130, 160], 1.0);
        let last = sim.stats.last().map(|s| s.tick).unwrap_or(1).max(1) as f32;
        for f in 0..n {
            let mut prev: Option<(f32, f32)> = None;
            for s in &sim.stats {
                let x = gx + 2.0 + (gw - 4.0) * s.tick as f32 / last;
                let y = gy + graph_h - 2.0 - (graph_h - 18.0) * get(s, f) as f32 / max;
                if let Some((x0, y0)) = prev {
                    c.line(x0, y0, x, y, 1.5, faction_color(f as i8), 0.9);
                }
                prev = Some((x, y));
            }
        }
    }
    // buttons
    let mut action = None;
    let labels: Vec<(&str, UiAction)> = if won {
        vec![("Retry", UiAction::Restart), ("Next level", UiAction::Next), ("Exit", UiAction::Levels)]
    } else {
        vec![("Retry", UiAction::Restart), ("Exit", UiAction::Levels)]
    };
    let bw = ((pw - 40.0 - 10.0 * (labels.len() - 1) as f32) / labels.len() as f32).min(200.0);
    let total = bw * labels.len() as f32 + 10.0 * (labels.len() - 1) as f32;
    let mut bx = view.0 / 2.0 - total / 2.0;
    let by = gy + graph_h + 24.0;
    for (label, a) in labels {
        if button(c, input, bx, by, bw, bh, label, false) {
            action = Some(a);
        }
        bx += bw + 10.0;
    }
    action
}

/// Controls help shown with F1.
pub fn draw_help(c: &mut Canvas) {
    let lines = [
        "CONTROLS",
        "",
        "Left drag ....................... draw a circle: select your ships inside it",
        "   (circling, hovering and flying ships alike; Shift adds to the selection)",
        "Left click a sun (nothing selected)  select the area around that sun",
        "Left / right click with a selection  send the selected ships there:",
        "   your sun = reinforce/upgrade, empty sun = capture, enemy sun = attack,",
        "   wormhole / white hole / supernova, or empty space = stop and hover",
        "Esc ............................. deselect (again: menu)",
        "Ctrl+A .......................... select all your ships",
        "WASD / arrows / middle-drag ..... pan      Wheel: zoom      F: fit",
        "Space or P ...................... pause     + / - : game speed (slow/normal/fast)",
        "Bars on a sun: health (green) and growth to the next level (white).",
        "",
        "Press F1 to close",
    ];
    let w = 720.0;
    let h = 24.0 + lines.len() as f32 * 16.0;
    let x = c.w as f32 / 2.0 - w / 2.0;
    let y = c.h as f32 / 2.0 - h / 2.0;
    c.rect(x, y, w, h, [10, 14, 30], 0.95);
    c.rect_outline(x, y, w, h, [90, 110, 160], 1.0);
    for (i, l) in lines.iter().enumerate() {
        c.text(x + 16.0, y + 14.0 + i as f32 * 16.0, l, 1, if i == 0 { [120, 200, 255] } else { WHITE }, 1.0);
    }
}
