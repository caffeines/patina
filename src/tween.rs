//! Small tweening engine: CSS-style transitions with cubic-bezier easing.
//!
//! Every animated value is keyed by an `Id`. When the target changes, the
//! tween restarts from wherever the value currently is, exactly like a CSS
//! `transition` does when a property changes mid-flight.

use std::collections::HashMap;
use std::hash::Hash;

use egui::{Color32, Id};

/// A CSS `cubic-bezier(x1, y1, x2, y2)` timing function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ease(pub f32, pub f32, pub f32, pub f32);

impl Ease {
    pub const LINEAR: Ease = Ease(0.0, 0.0, 1.0, 1.0);
    /// `ease`
    pub const EASE: Ease = Ease(0.25, 0.1, 0.25, 1.0);
    pub const EASE_OUT: Ease = Ease(0.0, 0.0, 0.58, 1.0);
    pub const EASE_IN_OUT: Ease = Ease(0.42, 0.0, 0.58, 1.0);
    /// The design's entrance curve, `cubic-bezier(.2,.8,.2,1)`.
    pub const OUT: Ease = Ease(0.2, 0.8, 0.2, 1.0);
    /// Material standard curve, `cubic-bezier(.4,0,.2,1)`.
    pub const STD: Ease = Ease(0.4, 0.0, 0.2, 1.0);
    /// Springy overshoot used by the nav highlight.
    pub const SPRING: Ease = Ease(0.3, 1.25, 0.4, 1.0);
    /// Springy overshoot used by segmented controls and tabs.
    pub const SPRING_SOFT: Ease = Ease(0.3, 1.2, 0.4, 1.0);
    /// Springy overshoot used by switches.
    pub const SPRING_HARD: Ease = Ease(0.3, 1.3, 0.4, 1.0);
    /// Pop-in curve used by dialogs and toasts.
    pub const POP: Ease = Ease(0.2, 0.9, 0.3, 1.1);
    /// The onboarding pie.
    pub const PIE: Ease = Ease(0.3, 0.8, 0.3, 1.0);

    pub fn at(self, t: f32) -> f32 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        if self == Ease::LINEAR {
            return t;
        }
        let Ease(x1, y1, x2, y2) = self;
        let bx = |s: f32| 3.0 * (1.0 - s) * (1.0 - s) * s * x1 + 3.0 * (1.0 - s) * s * s * x2 + s * s * s;
        let by = |s: f32| 3.0 * (1.0 - s) * (1.0 - s) * s * y1 + 3.0 * (1.0 - s) * s * s * y2 + s * s * s;
        let dx = |s: f32| {
            3.0 * (1.0 - s) * (1.0 - s) * x1 + 6.0 * (1.0 - s) * s * (x2 - x1) + 3.0 * s * s * (1.0 - x2)
        };
        // Newton's method, with a bisection fallback.
        let mut s = t;
        let mut ok = false;
        for _ in 0..8 {
            let e = bx(s) - t;
            if e.abs() < 1e-5 {
                ok = true;
                break;
            }
            let d = dx(s);
            if d.abs() < 1e-6 {
                break;
            }
            s = (s - e / d).clamp(0.0, 1.0);
        }
        if !ok {
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            s = t;
            for _ in 0..40 {
                let x = bx(s);
                if (x - t).abs() < 1e-5 {
                    break;
                }
                if x < t {
                    lo = s;
                } else {
                    hi = s;
                }
                s = (lo + hi) * 0.5;
            }
        }
        by(s)
    }
}

#[derive(Clone, Copy)]
struct Tw {
    from: [f32; 4],
    to: [f32; 4],
    start: f64,
    dur: f32,
    ease: Ease,
}

impl Tw {
    fn value(&self, now: f64) -> [f32; 4] {
        let t = if self.dur <= 0.0 { 1.0 } else { ((now - self.start) as f32 / self.dur).clamp(0.0, 1.0) };
        let e = self.ease.at(t);
        let mut out = [0.0; 4];
        for i in 0..4 {
            out[i] = self.from[i] + (self.to[i] - self.from[i]) * e;
        }
        out
    }
}

#[derive(Default)]
pub struct Tweens {
    map: HashMap<Id, Tw>,
    pub now: f64,
    /// Set when any tween (or entrance animation) is still moving this frame.
    pub busy: bool,
}

impl Tweens {
    pub fn begin_frame(&mut self, now: f64) {
        self.now = now;
        self.busy = false;
    }

    fn go(&mut self, id: Id, target: [f32; 4], dur: f32, ease: Ease) -> [f32; 4] {
        let now = self.now;
        let tw = self.map.entry(id).or_insert(Tw { from: target, to: target, start: now - 1e3, dur, ease });
        if tw.to != target {
            let cur = tw.value(now);
            *tw = Tw { from: cur, to: target, start: now, dur, ease };
        }
        let v = tw.value(now);
        let moving = now - tw.start < tw.dur as f64;
        if moving {
            self.busy = true;
        }
        v
    }

    pub fn f(&mut self, id: impl Hash, target: f32, dur: f32, ease: Ease) -> f32 {
        self.go(Id::new(id), [target, 0.0, 0.0, 0.0], dur, ease)[0]
    }

    pub fn c(&mut self, id: impl Hash, target: Color32, dur: f32) -> Color32 {
        let [r, g, b, a] = target.to_srgba_unmultiplied();
        let v = self.go(Id::new(id), [r as f32, g as f32, b as f32, a as f32], dur, Ease::EASE);
        Color32::from_rgba_unmultiplied(
            v[0].round().clamp(0.0, 255.0) as u8,
            v[1].round().clamp(0.0, 255.0) as u8,
            v[2].round().clamp(0.0, 255.0) as u8,
            v[3].round().clamp(0.0, 255.0) as u8,
        )
    }
}

/// CSS `@keyframes rise` (fade in, move up 10px) with a delay, measured from `t0`.
/// Returns `(alpha, dy, still_running)`.
pub fn rise(now: f64, t0: f64, delay: f32, dur: f32, dist: f32) -> (f32, f32, bool) {
    let t = ((now - t0) as f32 - delay) / dur;
    if t >= 1.0 {
        (1.0, 0.0, false)
    } else if t <= 0.0 {
        (0.0, dist, true)
    } else {
        let e = Ease::OUT.at(t);
        (e, dist * (1.0 - e), true)
    }
}
