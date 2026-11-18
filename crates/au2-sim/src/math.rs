//! Deterministic math: only IEEE basic operations and `sqrt` (all correctly rounded), plus a hand-written
//! sin/cos so results never depend on a platform libm. Seeded splitmix64 RNG.

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }
    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }
    pub fn len2(self) -> f64 {
        self.x * self.x + self.y * self.y
    }
    pub fn len(self) -> f64 {
        self.len2().sqrt()
    }
    pub fn dist(self, o: Vec2) -> f64 {
        (self - o).len()
    }
    pub fn dist2(self, o: Vec2) -> f64 {
        (self - o).len2()
    }
    /// Unit vector, or `fallback` when (nearly) zero.
    pub fn norm_or(self, fallback: Vec2) -> Vec2 {
        let l = self.len();
        if l < 1e-9 {
            fallback
        } else {
            self * (1.0 / l)
        }
    }
    /// Rotated a quarter turn counter-clockwise.
    pub fn perp(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }
    pub fn cross(self, o: Vec2) -> f64 {
        self.x * o.y - self.y * o.x
    }
    pub fn rotate(self, deg: f64) -> Vec2 {
        let (s, c) = (sin_deg(deg), cos_deg(deg));
        Vec2::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }
}
impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f64) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}
impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}
impl AddAssign for Vec2 {
    fn add_assign(&mut self, o: Vec2) {
        self.x += o.x;
        self.y += o.y;
    }
}

const PI: f64 = std::f64::consts::PI;

/// Sine of an angle in radians (Taylor series after range reduction to [-pi, pi]).
pub fn sin_rad(mut a: f64) -> f64 {
    // reduce by repeated subtraction of 2*pi multiples (inputs here are bounded, so use floor-free arithmetic)
    let two_pi = 2.0 * PI;
    let k = (a / two_pi + 0.5) as i64 as f64; // truncation toward zero is exact & deterministic
    a -= k * two_pi;
    if a > PI {
        a -= two_pi;
    } else if a < -PI {
        a += two_pi;
    }
    // fold into [-pi/2, pi/2] for accuracy
    if a > PI / 2.0 {
        a = PI - a;
    } else if a < -PI / 2.0 {
        a = -PI - a;
    }
    let x2 = a * a;
    let mut term = a;
    let mut sum = a;
    let mut n = 1.0;
    for _ in 0..9 {
        term = -term * x2 / ((n + 1.0) * (n + 2.0));
        sum += term;
        n += 2.0;
    }
    sum
}
pub fn cos_rad(a: f64) -> f64 {
    sin_rad(a + PI / 2.0)
}
pub fn sin_deg(d: f64) -> f64 {
    sin_rad(d * (PI / 180.0))
}
pub fn cos_deg(d: f64) -> f64 {
    cos_rad(d * (PI / 180.0))
}
/// Approximate `atan2(y, x)` in degrees, in (-180, 180]. Only basic arithmetic (deterministic everywhere); error is
/// below 0.3 degrees, which is plenty for placing ships on orbits.
pub fn atan2_deg(y: f64, x: f64) -> f64 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let (ax, ay) = (x.abs(), y.abs());
    // atan(t) for t in [0, 1]: pi/4*t + 0.273*t*(1-t)
    let (t, swap) = if ay > ax { (ax / ay, true) } else { (ay / ax, false) };
    let mut a = (PI / 4.0) * t + 0.273 * t * (1.0 - t);
    if swap {
        a = PI / 2.0 - a;
    }
    if x < 0.0 {
        a = PI - a;
    }
    if y < 0.0 {
        a = -a;
    }
    a * (180.0 / PI)
}

/// Unit vector at `deg` degrees.
pub fn dir_deg(d: f64) -> Vec2 {
    Vec2::new(cos_deg(d), sin_deg(d))
}

/// splitmix64: tiny, fast, good enough, trivially portable.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng { state: seed ^ 0x9E37_79B9_7F4A_7C15 }
    }
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }
    /// Uniform integer in 0..n (n > 0). Slight modulo bias is irrelevant here and deterministic.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.next_u32() % n
        }
    }
    /// Uniform in [-1, 1).
    pub fn signed_unit(&mut self) -> f64 {
        (self.next_u32() as f64) / 2147483648.0 - 1.0
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u32() as f64) / 4294967296.0
    }
    /// True with probability `ppm` / 1_000_000.
    pub fn chance_ppm(&mut self, ppm: u32) -> bool {
        self.below(1_000_000) < ppm
    }
    pub fn state(&self) -> u64 {
        self.state
    }
}
