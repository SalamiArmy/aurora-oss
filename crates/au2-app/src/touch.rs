//! Touch gestures, translated into the same `Input` the mouse produces so the game logic has one set of semantics.
//!
//! * tap = a left click (select the area around a sun / send the selection to the tapped target)
//! * one finger dragging = draw a lasso circle around ships to select them (a left press-drag-release)
//! * two fingers = pan the camera and pinch zoom
//!
//! All coordinates are in canvas (logical) pixels.

use crate::game::Input;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Idle,
    /// Finger down, not yet decided: a tap if it lifts without moving, a pan if it moves.
    Pending,
    /// Press already sent: follows the finger as a lasso (a tap if it lifts without moving).
    Select,
    Pinch,
    /// Waiting for every finger to lift (a tap that turned into a drag over a button, or one finger of a pinch lifted).
    Ignored,
}

pub struct Touch {
    fingers: Vec<(u64, (f32, f32))>,
    mode: Mode,
    start: (f32, f32),
    pinch_dist: f32,
    pinch_mid: (f32, f32),
    start_in_ui: bool,
    pub slop: f32,
}

impl Default for Touch {
    fn default() -> Self {
        Touch { fingers: Vec::new(), mode: Mode::Idle, start: (0.0, 0.0), pinch_dist: 1.0, pinch_mid: (0.0, 0.0), start_in_ui: false, slop: 12.0 }
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

impl Touch {
    /// True while at least one finger is on the screen.
    pub fn active(&self) -> bool {
        !self.fingers.is_empty()
    }

    /// Feed one touch event. `in_ui`: the first finger landed on the bottom bar, a menu or an overlay (tap only,
    /// never draws a lasso).
    pub fn event(&mut self, phase: Phase, id: u64, pos: (f32, f32), input: &mut Input, in_ui: bool) {
        input.touch_mode = true;
        match phase {
            Phase::Started => {
                if self.fingers.iter().any(|f| f.0 == id) {
                    return;
                }
                self.fingers.push((id, pos));
                match self.fingers.len() {
                    1 => {
                        self.start = pos;
                        self.start_in_ui = in_ui;
                        input.mouse = pos;
                        if !in_ui {
                            self.mode = Mode::Select;
                            input.left_down = true;
                            input.left_pressed = true;
                        } else {
                            self.mode = Mode::Pending;
                        }
                    }
                    2 => {
                        if self.mode == Mode::Select {
                            input.left_down = false;
                            input.cancel_drag = true;
                        }
                        let (a, b) = (self.fingers[0].1, self.fingers[1].1);
                        self.mode = if self.start_in_ui { Mode::Ignored } else { Mode::Pinch };
                        self.pinch_dist = dist(a, b).max(1.0);
                        self.pinch_mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                        input.mouse = self.pinch_mid;
                    }
                    _ => {}
                }
            }
            Phase::Moved => {
                let Some(f) = self.fingers.iter_mut().find(|f| f.0 == id) else { return };
                f.1 = pos;
                match self.mode {
                    Mode::Select => input.mouse = pos,
                    Mode::Pending => {
                        if dist(pos, self.start) > self.slop {
                            self.mode = Mode::Ignored;
                        }
                    }
                    Mode::Pinch if self.fingers.len() >= 2 => {
                        let (a, b) = (self.fingers[0].1, self.fingers[1].1);
                        let d = dist(a, b).max(1.0);
                        let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                        input.pinch = (if input.pinch > 0.0 { input.pinch } else { 1.0 }) * d / self.pinch_dist;
                        input.pan_px.0 += mid.0 - self.pinch_mid.0;
                        input.pan_px.1 += mid.1 - self.pinch_mid.1;
                        self.pinch_dist = d;
                        self.pinch_mid = mid;
                        input.mouse = mid;
                    }
                    _ => {}
                }
            }
            Phase::Ended | Phase::Cancelled => {
                let Some(k) = self.fingers.iter().position(|f| f.0 == id) else { return };
                self.fingers.remove(k);
                if !self.fingers.is_empty() {
                    self.mode = Mode::Ignored;
                    return;
                }
                match (self.mode, phase) {
                    (Mode::Select, Phase::Ended) => {
                        input.mouse = pos;
                        input.left_down = false;
                        input.left_released = true;
                    }
                    (Mode::Select, _) => {
                        input.left_down = false;
                        input.cancel_drag = true;
                    }
                    (Mode::Pending, Phase::Ended) => {
                        input.mouse = pos;
                        input.left_pressed = true;
                        input.left_released = true;
                    }
                    _ => {}
                }
                self.mode = Mode::Idle;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_is_a_click() {
        let (mut t, mut i) = (Touch::default(), Input::default());
        t.event(Phase::Started, 1, (50.0, 60.0), &mut i, false);
        assert!(i.left_pressed);
        t.event(Phase::Ended, 1, (50.0, 60.0), &mut i, false);
        assert!(i.left_released);
        assert_eq!(i.mouse, (50.0, 60.0));
        assert!(i.touch_mode && !t.active());
    }

    #[test]
    fn one_finger_drag_anywhere_is_a_lasso() {
        let (mut t, mut i) = (Touch::default(), Input::default());
        t.event(Phase::Started, 1, (100.0, 100.0), &mut i, false);
        assert!(i.left_pressed);
        t.event(Phase::Moved, 1, (200.0, 180.0), &mut i, false);
        assert_eq!(i.mouse, (200.0, 180.0));
        t.event(Phase::Ended, 1, (200.0, 180.0), &mut i, false);
        assert!(i.left_released && i.pan_px == (0.0, 0.0));
    }

    #[test]
    fn two_fingers_pan_and_pinch_and_cancel_the_lasso() {
        let (mut t, mut i) = (Touch::default(), Input::default());
        t.event(Phase::Started, 1, (100.0, 100.0), &mut i, false);
        t.event(Phase::Started, 2, (200.0, 100.0), &mut i, false);
        assert!(i.cancel_drag);
        t.event(Phase::Moved, 2, (300.0, 100.0), &mut i, false);
        assert!((i.pinch - 2.0).abs() < 1e-4, "{}", i.pinch);
        assert_eq!(i.pan_px, (50.0, 0.0), "the midpoint moved: pan");
        t.event(Phase::Ended, 2, (300.0, 100.0), &mut i, false);
        t.event(Phase::Ended, 1, (100.0, 100.0), &mut i, false);
        assert!(!i.left_released, "lifting after a pinch is not a click");
    }

    #[test]
    fn bar_taps_work_but_never_pan() {
        let (mut t, mut i) = (Touch::default(), Input::default());
        t.event(Phase::Started, 1, (10.0, 500.0), &mut i, true);
        t.event(Phase::Moved, 1, (90.0, 500.0), &mut i, true);
        t.event(Phase::Ended, 1, (90.0, 500.0), &mut i, true);
        assert!(i.pan_px == (0.0, 0.0) && !i.left_pressed);
    }
}
