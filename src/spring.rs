//! Spring-based animation, following the motion spec.
//!
//! Every animated value keeps its own position *and* velocity, so changing the
//! target mid-flight continues smoothly from where it is instead of restarting.

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub k: f32,
    pub c: f32,
}

/// Next / previous image, swipe. Firm, no overshoot.
pub const SWIPE: Spring = Spring { k: 380.0, c: 34.0 };
/// Zoom and pan. Soft follow with a tiny overshoot.
pub const ZOOM: Spring = Spring { k: 260.0, c: 26.0 };
/// Info panel open / close. Pops out with one small bounce.
pub const PANEL: Spring = Spring { k: 420.0, c: 24.0 };
/// Filmstrip selection. The snappiest one.
pub const THUMB: Spring = Spring { k: 520.0, c: 26.0 };

#[derive(Clone, Copy, Debug)]
pub struct Anim {
    pub x: f32,
    pub v: f32,
    pub to: f32,
}

impl Anim {
    pub const fn new(x: f32) -> Self {
        Self { x, v: 0.0, to: x }
    }

    /// Jump to a value immediately, without animating.
    pub fn snap(&mut self, x: f32) {
        self.x = x;
        self.to = x;
        self.v = 0.0;
    }

    /// Advance by `dt` seconds. Returns true while still moving.
    pub fn step(&mut self, s: Spring, dt: f32, eps: f32) -> bool {
        if self.settled(eps) {
            self.x = self.to;
            self.v = 0.0;
            return false;
        }
        // Fixed sub-steps keep the integration stable on slow frames.
        const H: f32 = 1.0 / 480.0;
        let mut left = dt.min(0.1);
        while left > 0.0 {
            let h = left.min(H);
            let a = -s.k * (self.x - self.to) - s.c * self.v;
            self.v += a * h;
            self.x += self.v * h;
            left -= h;
        }
        true
    }

    pub fn settled(&self, eps: f32) -> bool {
        (self.x - self.to).abs() < eps && self.v.abs() < eps * 10.0
    }
}
