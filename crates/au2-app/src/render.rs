//! Drawing the game and its HUD onto the software canvas.

use crate::game::{Game, Input, Overlay, BAR_H};
use crate::gfx::{mix_rgb, scale_rgb, Canvas, Rgb};
use au2_map::TerrainKind;
use au2_sim::math::Vec2;
use au2_sim::rules::{CAPTURE_COST, GAMMA_HUB, UPGRADE_COST};
use au2_sim::{FragilePhase, Obj, Status, Target};

pub const FACTION_COLORS: [Rgb; 4] = [[80, 200, 255], [255, 120, 70], [110, 230, 110], [235, 110, 235]];
pub const NEUTRAL: Rgb = [150, 150, 165];
pub const WHITE: Rgb = [235, 240, 255];

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
    let h = c.h as f32;
    for y in 0..c.h {
        let t = y as f32 / h;
        let col = [(6.0 + 8.0 * t) as u8, (8.0 + 8.0 * t) as u8, (18.0 + 14.0 * t) as u8];
        let p = ((col[0] as u32) << 16) | ((col[1] as u32) << 8) | col[2] as u32;
        let row = (y * c.w) as usize;
        c.buf[row..row + c.w as usize].fill(p);
    }
    // fixed star field in screen space
    for i in 0..140u32 {
        let x = (hash(i * 2) % c.w.max(1) as u32) as i32;
        let y = (hash(i * 2 + 1) % c.h.max(1) as u32) as i32;
        let a = 0.15 + (hash(i + 999) % 100) as f32 / 400.0;
        c.blend(x, y, WHITE, a);
    }
}

fn screen_dir(deg: f32) -> (f32, f32) {
    let r = deg.to_radians();
    (r.cos(), -r.sin())
}

pub fn draw_game(c: &mut Canvas, g: &Game, input: &Input) {
    let view = (c.w as f32, c.h as f32);
    let z = g.zoom;
    let t = g.clock;
    let sim = &g.sim;
    let vision = sim.vision(0);
    let sp = |p: Vec2| g.to_screen(p, view);

    background(c);

    // ------------------------------------------------------------------ terrain
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
            let (cx, cy) = sp(Vec2::new(d.pos[0], d.pos[1]));
            c.disc(cx, cy, d.outer_radius as f32 * z, col, 0.10);
            c.ring(cx, cy, d.outer_radius as f32 * z, 1.5, col, 0.6);
            if d.inner_radius > 0.0 {
                c.ring(cx, cy, d.inner_radius as f32 * z, 1.5, col, 0.6);
            }
            continue;
        }
        for r in 0..d.rows {
            for cc in 0..d.cols {
                if d.data.get((r * d.cols + cc) as usize).copied().unwrap_or(0) == 0 {
                    continue;
                }
                let wx = d.pos[0] + cc as f64 * d.tile_size;
                let wy = d.pos[1] + (r + 1) as f64 * d.tile_size; // top edge in world (y up)
                let (sx, sy) = sp(Vec2::new(wx, wy));
                let sz = d.tile_size as f32 * z;
                if sx > view.0 || sy > view.1 || sx + sz < 0.0 || sy + sz < 0.0 {
                    continue;
                }
                let alpha = if d.kind == TerrainKind::Reflect { 0.55 } else { 0.13 };
                c.rect(sx, sy, sz, sz, col, alpha);
                match d.kind {
                    TerrainKind::Force => {
                        let a = tr.accel;
                        let l = a.len().max(1e-6);
                        let (ux, uy) = ((a.x / l) as f32, -(a.y / l) as f32);
                        let (mx, my) = (sx + sz / 2.0, sy + sz / 2.0);
                        let ph = (t * 0.8 + (r + cc) as f32 * 0.3).fract() - 0.5;
                        let (px, py) = (mx + ux * ph * sz * 0.6, my + uy * ph * sz * 0.6);
                        c.line(px - ux * sz * 0.2, py - uy * sz * 0.2, px + ux * sz * 0.2, py + uy * sz * 0.2, 1.5, col, 0.6);
                        c.disc(px + ux * sz * 0.2, py + uy * sz * 0.2, 2.0, col, 0.8);
                    }
                    TerrainKind::Minefield => {
                        for k in 0..3u32 {
                            let h = hash(r * 131 + cc * 17 + k);
                            let (fx, fy) = ((h & 255) as f32 / 255.0, ((h >> 8) & 255) as f32 / 255.0);
                            let blink = 0.5 + 0.5 * (t * 3.0 + h as f32).sin();
                            c.disc(sx + fx * sz, sy + fy * sz, 1.5, col, 0.4 + 0.5 * blink);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ------------------------------------------------------------------ gamma rays and shockwaves
    for o in &sim.objs {
        match o {
            Obj::Gamma(gm) => {
                let (ox, oy) = sp(gm.pos);
                let (dx, dy) = screen_dir(gm.angle as f32);
                let (nx, ny) = (-dy, dx);
                let hw = gm.def.width as f32 / 2.0 * z;
                let (a0, a1) = (GAMMA_HUB as f32 * z, 30000.0 * z);
                let pts = [
                    (ox + dx * a0 + nx * hw, oy + dy * a0 + ny * hw),
                    (ox + dx * a1 + nx * hw, oy + dy * a1 + ny * hw),
                    (ox + dx * a1 - nx * hw, oy + dy * a1 - ny * hw),
                    (ox + dx * a0 - nx * hw, oy + dy * a0 - ny * hw),
                ];
                let col: Rgb = if gm.def.kill_units { [255, 60, 60] } else { [255, 170, 60] };
                c.convex(&pts, col, 0.16);
                c.line(ox + dx * a0, oy + dy * a0, ox + dx * a1.min(4000.0), oy + dy * a1.min(4000.0), 2.0, col, 0.5);
                c.glow(ox, oy, 60.0 * z.max(0.4) + 20.0, col, 0.6);
                c.ring(ox, oy, GAMMA_HUB as f32 * z, 1.0, col, 0.35);
                c.disc(ox, oy, 6.0, WHITE, 0.9);
            }
            Obj::Shock(sh) => {
                let d = &sh.def;
                if sim.tick >= d.dormant as u64 && d.period > 0 {
                    let age = (sim.tick - d.dormant as u64) % d.period as u64;
                    if age < d.lifetime as u64 {
                        let reach = (d.speed * age as f64 + d.start_offset) as f32;
                        let (ox, oy) = sp(sh.pos);
                        if d.kind == "st_incircle" {
                            c.ring(ox, oy, reach * z, 3.0, [255, 230, 140], 0.6);
                        } else {
                            let r = d.line_rotation_deg.to_radians() as f32;
                            let (ux, uy) = (-r.sin(), r.cos());
                            let at = reach - 12000.0;
                            let (cx, cy) = (ox + ux * at * z, oy - uy * at * z);
                            let (tx, ty) = (r.cos(), -r.sin());
                            c.line(cx - tx * 30000.0 * z, cy - ty * 30000.0 * z, cx + tx * 30000.0 * z, cy + ty * 30000.0 * z, 3.0, [255, 230, 140], 0.6);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ orbit paths
    for o in &sim.objs {
        if let Obj::Sun(s) = o {
            if let Some(or) = &s.orbit {
                let rot = or.rotation_deg.to_radians() as f32;
                let mut prev: Option<(f32, f32)> = None;
                for i in 0..=72 {
                    let a = i as f32 / 72.0 * std::f32::consts::TAU;
                    let (ex, ey) = (or.rx as f32 * a.cos(), or.ry as f32 * a.sin());
                    let (wx, wy) = (ex * rot.cos() - ey * rot.sin(), ex * rot.sin() + ey * rot.cos());
                    let p = sp(Vec2::new(s.centre.x + wx as f64, s.centre.y + wy as f64));
                    if let Some(q) = prev {
                        if i % 2 == 0 {
                            c.line(q.0, q.1, p.0, p.1, 1.0, NEUTRAL, 0.18);
                        }
                    }
                    prev = Some(p);
                }
            }
        }
    }

    // ------------------------------------------------------------------ wormholes, holes, supernovae
    for (i, o) in sim.objs.iter().enumerate() {
        match o {
            Obj::Wormhole(w) => {
                let (x, y) = sp(w.pos);
                let r = w.size as f32 * z;
                let col: Rgb = [150, 120, 255];
                c.glow(x, y, r * 2.0, col, 0.35);
                c.disc(x, y, r, [10, 6, 30], 0.85);
                for k in 0..3 {
                    let rr = r * (0.9 - 0.25 * k as f32);
                    let dir = if k % 2 == 0 { 1.0 } else { -1.0 };
                    c.arc(x, y, rr, 2.0, t * dir * (1.0 + k as f32 * 0.5) + k as f32, 3.6, col, 0.8);
                }
                c.ring(x, y, r, 1.5, col, 0.8);
                if w.dest > i as i32 {
                    if let Some(p) = sim.obj_pos(w.dest as usize) {
                        let q = sp(p);
                        c.dashed_line(x, y, q.0, q.1, 8.0, col, 0.18);
                    }
                }
            }
            Obj::WhiteHole(w) => {
                let (x, y) = sp(w.pos);
                let r = (w.def.game_size as f32 * z).max(10.0);
                let dormant = sim.tick < w.dormant_until;
                let a = if dormant { 0.35 } else { 1.0 };
                c.glow(x, y, r * 2.5, WHITE, 0.4 * a);
                c.disc(x, y, r * 0.6, WHITE, 0.9 * a);
                c.ring(x, y, r, 2.0, WHITE, 0.8 * a);
                c.arc(x, y, r * 1.25, 3.0, -1.57, w.invest[0] as f32 / w.def.required_investment.max(1) as f32 * std::f32::consts::TAU, FACTION_COLORS[0], 0.9);
                c.text_centered(x, y + r * 1.5 + 4.0, &format!("{}/{}", w.invest[0], w.def.required_investment), 1, WHITE, 0.8);
            }
            Obj::Supernova(s) => {
                let (x, y) = sp(s.pos);
                let r = (s.def.start_scale as f32 * z).max(10.0);
                let dormant = sim.tick < s.dormant_until;
                let pulse = 1.0 + 0.1 * (t * 4.0).sin();
                let col: Rgb = [255, 170, 60];
                let a = if dormant { 0.35 } else { 1.0 };
                c.glow(x, y, r * 3.0 * pulse, col, 0.45 * a);
                c.disc(x, y, r * 0.55 * pulse, [255, 230, 160], 0.9 * a);
                c.ring(x, y, r, 2.0, col, 0.8 * a);
                c.arc(x, y, r * 1.3, 3.0, -1.57, s.invest[0] as f32 / s.def.required_investment.max(1) as f32 * std::f32::consts::TAU, FACTION_COLORS[0], 0.9);
                c.text_centered(x, y + r * 1.6 + 4.0, &format!("{}/{}", s.invest[0], s.def.required_investment), 1, WHITE, 0.8);
                if let Some(lb) = s.last_blast {
                    let age = (sim.tick - lb) as f32 / 60.0;
                    if age < 1.0 {
                        c.ring(x, y, s.def.blast_radius as f32 * z * age, 4.0, col, 1.0 - age);
                    }
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ suns
    for (i, s) in sim.suns() {
        let (x, y) = sp(s.pos);
        let base_r = s.radius as f32 * z;
        let r = base_r.max(7.0);
        if x < -r * 3.0 || y < -r * 3.0 || x > view.0 + r * 3.0 || y > view.1 + r * 3.0 {
            continue;
        }
        let seen = vision.visible(s.pos) || s.faction == 0;
        if let Some(fr) = &s.fragile {
            match fr.phase {
                FragilePhase::Gone { .. } => {
                    c.ring(x, y, r, 1.0, NEUTRAL, 0.2);
                    continue;
                }
                FragilePhase::Rebuilding { from, until } => {
                    let k = (sim.tick - from) as f32 / (until - from).max(1) as f32;
                    c.disc(x, y, r * k, NEUTRAL, 0.3);
                    c.ring(x, y, r, 1.0, NEUTRAL, 0.35);
                    continue;
                }
                FragilePhase::Intact => {}
            }
        }
        if !seen {
            c.ring(x, y, r, 1.5, NEUTRAL, 0.35);
            c.text_centered(x, y - 4.0, "?", 1, NEUTRAL, 0.6);
            continue;
        }
        let col = faction_color(s.faction);
        let owned = s.faction >= 0;
        c.glow(x, y, r * 2.4, col, if owned { 0.30 + 0.05 * s.level as f32 } else { 0.10 });
        c.disc(x, y, r, scale_rgb(col, 0.32), 1.0);
        c.disc(x, y, r * 0.86, scale_rgb(col, 0.55), 1.0);
        c.disc(x, y, r * 0.58, scale_rgb(col, if owned { 0.9 } else { 0.6 }), 0.9);
        for k in 0..3 {
            let dir = if k % 2 == 0 { 1.0 } else { -1.0 };
            let rr = r * (0.30 + 0.22 * k as f32);
            let a0 = t * dir * (0.7 + 0.45 * k as f32) + k as f32 * 2.1 + i as f32;
            c.arc(x, y, rr, (r * 0.07).max(1.5), a0, 1.5, mix_rgb(col, WHITE, 0.6), if owned { 0.75 } else { 0.35 });
        }
        c.ring(x, y, r, 1.5, col, 0.9);
        if s.fragile.is_some() {
            c.line(x - r * 0.5, y - r * 0.3, x + r * 0.2, y + r * 0.45, 1.0, WHITE, 0.35);
            c.line(x + r * 0.1, y - r * 0.6, x - r * 0.1, y + r * 0.1, 1.0, WHITE, 0.35);
        }
        // progress ring
        if owned && s.level < s.max_level {
            let frac = s.progress as f32 / UPGRADE_COST[s.level as usize] as f32;
            c.ring(x, y, r + 5.0, 2.0, col, 0.18);
            c.arc(x, y, r + 5.0, 3.0, -std::f32::consts::FRAC_PI_2, frac.clamp(0.0, 1.0) * std::f32::consts::TAU, WHITE, 0.9);
        } else if !owned && s.cap_progress > 0 {
            let frac = s.cap_progress as f32 / CAPTURE_COST as f32;
            c.arc(x, y, r + 5.0, 3.0, -std::f32::consts::FRAC_PI_2, frac.clamp(0.0, 1.0) * std::f32::consts::TAU, faction_color(s.cap_faction), 0.9);
        }
        // level pips
        let n = s.max_level as i32;
        for k in 0..n {
            let px = x + (k as f32 - (n - 1) as f32 / 2.0) * 7.0;
            let py = y + r + 14.0;
            if (k as u8) < s.level {
                c.disc(px, py, 2.5, col, 1.0);
            } else {
                c.ring(px, py, 2.5, 1.0, col, 0.5);
            }
        }
        if owned {
            let txt = format!("{}", s.stock);
            let sc = if r > 20.0 { 2 } else { 1 };
            c.text_centered(x, y - 4.0 * sc as f32, &txt, sc, WHITE, 0.95);
        }
        if let Some(rp) = &s.repeat {
            if let Some(tp) = sim.target_pos(rp.target) {
                let q = sp(tp);
                c.dashed_line(x, y, q.0, q.1, 10.0, col, 0.35);
            }
        }
        if g.selection.contains(&i) {
            let pulse = 0.7 + 0.3 * (t * 6.0).sin();
            c.ring(x, y, r + 10.0, 2.0, WHITE, pulse);
        }
    }

    // ------------------------------------------------------------------ ships
    for s in &sim.ships {
        if s.faction != 0 && !vision.visible(s.pos) {
            continue;
        }
        let (x, y) = sp(s.pos);
        if x < -20.0 || y < -20.0 || x > view.0 + 20.0 || y > view.1 + 20.0 {
            continue;
        }
        let col = faction_color(s.faction as i8);
        let moving = s.target != Target::None;
        if moving {
            let tl = 7.0;
            c.line(x, y, x - s.heading.x as f32 * tl, y + s.heading.y as f32 * tl, 1.0, col, 0.45);
        }
        let b = mix_rgb(col, WHITE, 0.5);
        c.add(x.floor() as i32, y.floor() as i32, b, 1.0);
        c.add(x.floor() as i32 + 1, y.floor() as i32, col, 0.9);
        c.add(x.floor() as i32, y.floor() as i32 + 1, col, 0.9);
        c.add(x.floor() as i32 + 1, y.floor() as i32 + 1, col, 0.7);
    }

    // ------------------------------------------------------------------ effects
    for sp_ in &g.sparks {
        let (x, y) = sp(sp_.pos);
        let k = sp_.age / 0.5;
        c.ring(x, y, 2.0 + k * 10.0, 1.5, faction_color(sp_.color_faction as i8), (1.0 - k) * 0.9);
    }
    for p in &g.pings {
        let (x, y) = sp(p.pos);
        let k = p.age / 0.8;
        c.ring(x, y, 6.0 + k * 30.0, 2.0, WHITE, 1.0 - k);
    }

    // ------------------------------------------------------------------ selection feedback
    let modal = g.overlay != Overlay::None || sim.status != Status::Playing;
    if !modal && !g.selection.is_empty() && input.mouse.1 < view.1 - BAR_H {
        let wp = g.to_world(input.mouse, view);
        let hover = sim.pick(wp, (6.0 / z) as f64);
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
            None => (input.mouse.0, input.mouse.1, WHITE),
        };
        for &i in &g.selection {
            if let Some(s) = sim.sun(i) {
                let (x, y) = sp(s.pos);
                c.dashed_line(x, y, tx, ty, 8.0, col, 0.55);
            }
        }
    }
    if let Some(d0) = g.drag {
        if (input.mouse.0 - d0.0).abs() + (input.mouse.1 - d0.1).abs() > 8.0 {
            let (x0, y0) = (d0.0.min(input.mouse.0), d0.1.min(input.mouse.1));
            let (w, h) = ((d0.0 - input.mouse.0).abs(), (d0.1 - input.mouse.1).abs());
            c.rect(x0, y0, w, h, FACTION_COLORS[0], 0.10);
            c.rect_outline(x0, y0, w, h, FACTION_COLORS[0], 0.8);
        }
    }

    fog_overlay(c, g, &vision, view);
}

/// Darken everything the human cannot see (levels with fog of war).
fn fog_overlay(c: &mut Canvas, g: &Game, vision: &au2_sim::Vision, view: (f32, f32)) {
    if vision.all {
        return;
    }
    const CELL: i32 = 4;
    let (cw, ch) = (c.w / CELL + 2, c.h / CELL + 2);
    let mut mask = vec![0.0f32; (cw * ch) as usize];
    for (centre, radius) in &vision.circles {
        let (sx, sy) = g.to_screen(*centre, view);
        let rad = *radius as f32 * g.zoom;
        let (cx, cy, cr) = (sx / CELL as f32, sy / CELL as f32, rad / CELL as f32);
        let (x0, x1) = (((cx - cr).floor() as i32 - 1).max(0), ((cx + cr).ceil() as i32 + 1).min(cw - 1));
        let (y0, y1) = (((cy - cr).floor() as i32 - 1).max(0), ((cy + cr).ceil() as i32 + 1).min(ch - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() / cr.max(0.001);
                let v = ((1.0 - d) / 0.2).clamp(0.0, 1.0);
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
    let mut action = None;
    let sim = &g.sim;

    // ------------------------------------------------------------------ top-left info
    c.rect(0.0, 0.0, 330.0, 74.0, [8, 10, 22], 0.6);
    c.text(10.0, 8.0, &format!("{} - {}", g.entry.constellation, g.entry.name), 2, WHITE, 1.0);
    let secs = g.seconds() as u32;
    c.text(10.0, 30.0, &format!("time {:02}:{:02}   AI level {}   x{}{}", secs / 60, secs % 60, g.entry.difficulty, g.speed, if g.paused { "  PAUSED" } else { "" }), 1, [180, 190, 220], 1.0);
    c.text(10.0, 44.0, &format!("send {}%{}   selected {}", g.percent, if g.repeat { " (repeat)" } else { "" }, g.selection.len()), 1, [180, 190, 220], 1.0);
    c.text(10.0, 58.0, "RMB/click target: order   drag: lasso   F1: help", 1, [120, 130, 160], 1.0);

    // ------------------------------------------------------------------ top-right faction table
    let n = sim.faction_count();
    let x0 = view.0 - 190.0;
    c.rect(x0 - 8.0, 0.0, 198.0, 14.0 + 16.0 * n as f32, [8, 10, 22], 0.6);
    let vis = sim.vision(0);
    for f in 0..n {
        let col = faction_color(f as i8);
        let y = 8.0 + 16.0 * f as f32;
        c.disc(x0, y + 4.0, 4.0, col, if sim.factions[f].alive { 1.0 } else { 0.25 });
        let ships = sim.ship_count(f as u8) as u32 + sim.stock_total(f as u8);
        let label = if f == 0 || vis.all {
            format!("{}: {} suns {} ships", if f == 0 { "you" } else { "AI" }, sim.sun_count(f as u8), ships)
        } else {
            format!("AI: ? suns ? ships")
        };
        c.text(x0 + 10.0, y, &label, 1, if sim.factions[f].alive { WHITE } else { [110, 110, 120] }, 1.0);
    }

    // ------------------------------------------------------------------ toast
    if let Some((msg, _)) = &g.toast {
        let w = Canvas::text_width(msg, 2) as f32 + 24.0;
        c.rect(view.0 / 2.0 - w / 2.0, view.1 - BAR_H - 44.0, w, 30.0, [10, 14, 30], 0.85);
        c.text_centered(view.0 / 2.0, view.1 - BAR_H - 36.0, msg, 2, WHITE, 1.0);
    }

    // ------------------------------------------------------------------ bottom bar
    let by = view.1 - BAR_H;
    c.rect(0.0, by, view.0, BAR_H, [12, 16, 30], 0.96);
    c.rect(0.0, by, view.0, 1.0, [70, 85, 120], 1.0);
    let (bh, y) = (30.0, by + 7.0);
    let mut x = 8.0;
    for (label, p) in [("25%", 25u8), ("50%", 50), ("100%", 100)] {
        if button(c, input, x, y, 58.0, bh, label, g.percent == p) {
            g.percent = p;
        }
        x += 62.0;
    }
    x += 10.0;
    if button(c, input, x, y, 90.0, bh, "Repeat", g.repeat) {
        g.repeat = !g.repeat;
    }
    x += 94.0;
    if button(c, input, x, y, 90.0, bh, "Cancel", false) && !g.selection.is_empty() {
        let sel = g.selection.clone();
        g.cmds.push(au2_sim::Command::Cancel { faction: 0, sources: sel });
        g.toast("standing orders cancelled");
    }
    x += 104.0;
    if button(c, input, x, y, 80.0, bh, if g.paused { "Play" } else { "Pause" }, g.paused) {
        g.paused = !g.paused;
    }
    x += 84.0;
    if button(c, input, x, y, 50.0, bh, &format!("x{}", g.speed), g.speed > 1) {
        g.speed = if g.speed >= 4 { 1 } else { g.speed * 2 };
    }
    x += 64.0;
    if button(c, input, x, y, 60.0, bh, "Fit", false) {
        g.fit(view);
    }
    if button(c, input, view.0 - 98.0, y, 90.0, bh, "Menu", false) {
        g.overlay = Overlay::Pause;
    }

    // ------------------------------------------------------------------ overlays
    let decided = g.sim.status != Status::Playing;
    if decided || g.overlay == Overlay::Pause {
        c.rect(0.0, 0.0, view.0, view.1, [0, 0, 8], 0.55);
        let (pw, ph) = (360.0, if decided { 250.0 } else { 300.0 });
        let (px, py) = (view.0 / 2.0 - pw / 2.0, view.1 / 2.0 - ph / 2.0);
        c.rect(px, py, pw, ph, [14, 18, 36], 0.97);
        c.rect_outline(px, py, pw, ph, [90, 110, 160], 1.0);
        let (title, tcol): (&str, Rgb) = match (decided, g.sim.status) {
            (true, Status::Won) => ("VICTORY", [120, 255, 150]),
            (true, _) => ("DEFEAT", [255, 110, 100]),
            _ => ("PAUSED", WHITE),
        };
        c.text_centered(view.0 / 2.0, py + 20.0, title, 4, tcol, 1.0);
        let mut by = py + 80.0;
        let bx = px + 40.0;
        let bw = pw - 80.0;
        if !decided && button(c, input, bx, by, bw, 34.0, "Resume", false) {
            action = Some(UiAction::Resume);
        }
        if !decided {
            by += 44.0;
        }
        if decided && g.sim.status == Status::Won && g.level_index + 1 < usize::MAX && button(c, input, bx, by, bw, 34.0, "Next level", false) {
            action = Some(UiAction::Next);
        }
        if decided && g.sim.status == Status::Won {
            by += 44.0;
        }
        if button(c, input, bx, by, bw, 34.0, "Restart level", false) {
            action = Some(UiAction::Restart);
        }
        by += 44.0;
        if !decided {
            if button(c, input, bx, by, bw, 34.0, "Surrender", false) {
                action = Some(UiAction::Surrender);
            }
            by += 44.0;
        }
        if button(c, input, bx, by, bw, 34.0, "Level select", false) {
            action = Some(UiAction::Levels);
        }
    }
    action
}

/// Controls help shown with F1.
pub fn draw_help(c: &mut Canvas) {
    let lines = [
        "CONTROLS",
        "",
        "Left click a sun of yours ....... select it (Shift adds)",
        "Left drag ....................... lasso-select your suns",
        "Left click elsewhere / Right click  send ships there",
        "   (a sun, a wormhole, a white hole, a supernova or empty space)",
        "1 / 2 / 3 ....................... send 25% / 50% / 100%",
        "T ............................... repeat: new ships keep flying to the target",
        "X ............................... cancel standing orders of selected suns",
        "Ctrl+A .......................... select all your suns",
        "WASD / arrows / middle-drag ..... pan      Wheel: zoom      F: fit",
        "Space or P ...................... pause     + / - : game speed",
        "Esc ............................. menu (restart, surrender, level select)",
        "Send ships to your own sun to upgrade it; empty or enemy suns are captured/attacked.",
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
