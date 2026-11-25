//! Painting helpers and custom widgets: buttons, switches, segmented
//! controls, status dots, rings, progress bars and the little animations
//! (sheen, shimmer, spinners) that give the design its life.

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use egui::epaint::{Mesh, PathShape, Shadow};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    pos2, vec2, Align2, Color32, CursorIcon, FontId, Galley, Id, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui,
};

use crate::icons::{self, Ink};
use crate::sim::St;
use crate::theme::*;
use crate::tween::{Ease, Tweens};

// ----------------------------------------------------------------- shapes

pub fn fill(p: &Painter, r: Rect, rad: f32, c: Color32) {
    p.rect_filled(r, rad, c);
}

/// `box-shadow: inset 0 0 0 1px` style edge.
pub fn edge(p: &Painter, r: Rect, rad: f32, c: Color32) {
    p.rect_stroke(r, rad, Stroke::new(1.0_f32, c), StrokeKind::Inside);
}

pub fn card(p: &Painter, r: Rect, rad: f32, bg: Color32, e: Color32) {
    fill(p, r, rad, bg);
    edge(p, r, rad, e);
}

pub fn shadow(p: &Painter, r: Rect, rad: f32, blur: f32, dy: f32, c: Color32) {
    let s = Shadow { offset: [0, dy as i8], blur: blur.min(255.0) as u8, spread: 0, color: c };
    p.add(s.as_shape(r, rad));
}

pub fn arc_pts(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    let n = (((a1 - a0).abs() * r) / 1.5).ceil().clamp(6.0, 256.0) as usize;
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

/// An open arc with round caps.
pub fn arc(p: &Painter, c: Pos2, r: f32, a0: f32, a1: f32, stroke: Stroke) {
    if (a1 - a0).abs() < 1e-3 {
        return;
    }
    let pts = arc_pts(c, r, a0, a1);
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    p.add(Shape::Path(PathShape::line(pts, stroke)));
    p.circle_filled(first, stroke.width * 0.5, stroke.color);
    p.circle_filled(last, stroke.width * 0.5, stroke.color);
}

/// A filled pie slice. Split in halves so each part stays convex (and anti-aliased).
pub fn pie(p: &Painter, c: Pos2, r: f32, a0: f32, sweep: f32, color: Color32) {
    if sweep <= 0.001 {
        return;
    }
    let mut start = a0;
    let mut left = sweep.min(TAU);
    while left > 0.0005 {
        let part = left.min(PI);
        let mut pts = vec![c];
        pts.extend(arc_pts(c, r, start - 0.004, start + part));
        p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        start += part;
        left -= part;
    }
}

/// Progress ring: track plus an arc from 12 o'clock, clockwise.
pub fn ring(p: &Painter, c: Pos2, r: f32, frac: f32, color: Color32, track: Color32, w: f32, rot: f32) {
    p.circle_stroke(c, r, Stroke::new(w, track));
    if frac > 0.002 {
        let a0 = -PI / 2.0 + rot;
        arc(p, c, r, a0, a0 + frac.min(1.0) * TAU, Stroke::new(w, color));
    }
}

/// CSS-style spinner: a faint ring whose top quarter is colored, rotating.
pub fn spin_ring(p: &Painter, c: Pos2, r: f32, w: f32, track: Color32, head: Color32, now: f64, period: f64) {
    p.circle_stroke(c, r, Stroke::new(w, track));
    let rot = ((now % period) / period) as f32 * TAU;
    let pts = arc_pts(c, r, -0.75 * PI + rot, -0.25 * PI + rot);
    p.add(Shape::Path(PathShape::line(pts, Stroke::new(w, head))));
}

pub fn dashed_circle(p: &Painter, c: Pos2, r: f32, stroke: Stroke, dash: f32, gap: f32, rot: f32) {
    let pts = arc_pts(c, r, rot, rot + TAU);
    p.extend(Shape::dashed_line(&pts, stroke, dash, gap));
}

pub fn rrect_outline(r: Rect, rad: f32) -> Vec<Pos2> {
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0);
    let mut pts = Vec::new();
    let corners = [
        (pos2(r.right() - rad, r.top() + rad), -PI / 2.0),
        (pos2(r.right() - rad, r.bottom() - rad), 0.0),
        (pos2(r.left() + rad, r.bottom() - rad), PI / 2.0),
        (pos2(r.left() + rad, r.top() + rad), PI),
    ];
    pts.push(pos2(r.left() + rad, r.top()));
    for (c, a) in corners {
        pts.extend(arc_pts(c, rad, a, a + PI / 2.0));
    }
    pts.push(pos2(r.left() + rad, r.top()));
    pts
}

pub fn dashed_rrect(p: &Painter, r: Rect, rad: f32, stroke: Stroke, dash: f32, gap: f32) {
    let pts = rrect_outline(r.shrink(stroke.width / 2.0), rad);
    p.extend(Shape::dashed_line(&pts, stroke, dash, gap));
}

/// Fill a rounded rect with a color computed per vertex, in vertical strips.
/// Used for gradients (sheen, shimmer).
pub fn shade_rrect(p: &Painter, r: Rect, rad: f32, step: f32, f: impl Fn(Pos2) -> Color32) {
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let mut xs = Vec::new();
    let mut x = r.left();
    while x < r.right() {
        xs.push(x);
        let in_corner = x < r.left() + rad || x > r.right() - rad;
        x += if in_corner { 1.0 } else { step };
    }
    xs.push(r.right());
    let yt = |x: f32| {
        let dx = if x < r.left() + rad {
            r.left() + rad - x
        } else if x > r.right() - rad {
            x - (r.right() - rad)
        } else {
            0.0
        };
        if dx > 0.0 {
            r.top() + rad - (rad * rad - dx * dx).max(0.0).sqrt()
        } else {
            r.top()
        }
    };
    let mut m = Mesh::default();
    for (i, &x) in xs.iter().enumerate() {
        let t = yt(x);
        let b = r.bottom() - (t - r.top());
        m.colored_vertex(pos2(x, t), f(pos2(x, t)));
        m.colored_vertex(pos2(x, b), f(pos2(x, b)));
        if i > 0 {
            let k = (i as u32) * 2;
            m.add_triangle(k - 2, k - 1, k);
            m.add_triangle(k - 1, k + 1, k);
        }
    }
    p.add(Shape::mesh(m));
}

/// The diagonal light sweep on primary buttons (`@keyframes sheen`).
pub fn sheen(p: &Painter, r: Rect, rad: f32, now: f64) {
    let period = 5.5;
    let ph = ((now % period) / period) as f32;
    if ph < 0.72 {
        return;
    }
    let e = Ease::EASE_IN_OUT.at((ph - 0.72) / 0.28);
    let shift = -1.3 + 3.6 * e; // translateX(-130%) → 230%
    let w = r.width();
    let center_x = r.left() + w * (0.5 + shift);
    let half = 0.12 * (w * 0.966 + r.height() * 0.259) / 0.966;
    let tilt = -0.268; // 105deg gradient: bands lean like '/'
    let mid_y = r.center().y;
    shade_rrect(p, r, rad, 3.0, |q| {
        let cx = center_x + (q.y - mid_y) * tilt;
        let d = ((q.x - cx).abs() / half).min(1.0);
        Color32::from_white_alpha(((1.0 - d) * 0.4 * 255.0) as u8)
    });
}

/// Skeleton placeholder with the moving shimmer.
pub fn skeleton(p: &Painter, r: Rect, rad: f32, now: f64) {
    let ph = ((now % 1.5) / 1.5) as f32; // background-position 0 → -200%
    let w = r.width().max(1.0);
    shade_rrect(p, r, rad, 4.0, |q| {
        let u = (((q.x - r.left()) / (2.0 * w)) + ph).rem_euclid(1.0);
        let k = 1.0 - (u - 0.5).abs() * 2.0;
        mix(SKEL_A, SKEL_B, k)
    });
}

pub fn bar(p: &Painter, r: Rect, frac: f32, color: Color32, track: Color32) {
    let rad = r.height() / 2.0 + 0.5;
    fill(p, r, rad, track);
    let w = r.width() * frac.clamp(0.0, 1.0);
    if w > 0.3 {
        fill(p, Rect::from_min_size(r.min, vec2(w.max(r.height()), r.height())), rad, color);
    }
}

// ------------------------------------------------------------------- text

pub fn job(text: &str, font: FontId, color: Color32, ls: f32) -> LayoutJob {
    let mut j = LayoutJob::default();
    j.append(text, 0.0, TextFormat { font_id: font, color, extra_letter_spacing: ls, ..Default::default() });
    j
}

pub fn g(ui: &Ui, text: &str, font: FontId, color: Color32) -> Arc<Galley> {
    ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font, color))
}

pub fn g_ls(ui: &Ui, text: &str, font: FontId, color: Color32, ls: f32) -> Arc<Galley> {
    ui.fonts(|f| f.layout_job(job(text, font, color, ls)))
}

pub fn g_elide(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut j = job(text, font, color, 0.0);
    j.wrap = TextWrapping { max_width: max_w.max(4.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.fonts(|f| f.layout_job(j))
}

pub fn g_wrap(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32, line_h: f32) -> Arc<Galley> {
    let mut j = job(text, font, color, 0.0);
    j.sections[0].format.line_height = Some(line_h);
    j.wrap.max_width = max_w.max(10.0);
    ui.fonts(|f| f.layout_job(j))
}

/// Draw a galley aligned to `anchor`.
pub fn put(p: &Painter, gal: &Arc<Galley>, anchor: Pos2, align: Align2) -> Rect {
    let r = align.anchor_size(anchor, gal.size());
    p.galley(r.min, gal.clone(), TEXT);
    r
}

/// Draw text with its left edge at `x`, vertically centered on `cy`.
pub fn text(p: &Painter, ui: &Ui, x: f32, cy: f32, s: &str, font: FontId, color: Color32) -> Rect {
    let gal = g(ui, s, font, color);
    put(p, &gal, pos2(x, cy), Align2::LEFT_CENTER)
}

pub fn text_right(p: &Painter, ui: &Ui, x: f32, cy: f32, s: &str, font: FontId, color: Color32) -> Rect {
    let gal = g(ui, s, font, color);
    put(p, &gal, pos2(x, cy), Align2::RIGHT_CENTER)
}

pub fn text_w(ui: &Ui, s: &str, font: FontId) -> f32 {
    g(ui, s, font, TEXT).size().x
}

// ---------------------------------------------------------------- buttons

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BtnKind {
    Cta(Color32),
    Ghost,
    Plain,
}

#[derive(Clone)]
pub struct Btn {
    pub label: String,
    pub kind: BtnKind,
    pub h: f32,
    pub pad: f32,
    pub radius: f32,
    pub font: FontId,
    pub color: Color32,
    pub border: Color32,
    pub icon: Option<(&'static str, Ink, f32)>,
    pub spin: bool,
    pub enabled: bool,
    pub sheen: bool,
    pub w: Option<f32>,
    /// Empty space reserved before the label (for a custom mark).
    pub lead: f32,
}

impl Btn {
    pub fn cta(label: impl Into<String>) -> Btn {
        Btn {
            label: label.into(),
            kind: BtnKind::Cta(COPPER),
            h: 34.0,
            pad: 16.0,
            radius: 9.0,
            font: semibold(13.5),
            color: CTA_INK,
            border: BTN_EDGE,
            icon: None,
            spin: false,
            enabled: true,
            sheen: false,
            w: None,
            lead: 0.0,
        }
    }
    pub fn ghost(label: impl Into<String>) -> Btn {
        Btn { kind: BtnKind::Ghost, pad: 14.0, font: medium(13.5), color: TEXT, ..Btn::cta(label) }
    }
    pub fn plain(label: impl Into<String>) -> Btn {
        Btn { kind: BtnKind::Plain, h: 30.0, pad: 10.0, radius: 8.0, font: sans(13.5), color: TEXT2, ..Btn::cta(label) }
    }
    pub fn icon(d: &'static str, ink: Ink) -> Btn {
        Btn { icon: Some((d, ink, 16.0)), w: Some(30.0), ..Btn::plain("") }
    }
    pub fn h(mut self, h: f32) -> Self {
        self.h = h;
        self
    }
    pub fn pad(mut self, pad: f32) -> Self {
        self.pad = pad;
        self
    }
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    pub fn font(mut self, f: FontId) -> Self {
        self.font = f;
        self
    }
    pub fn color(mut self, c: Color32) -> Self {
        self.color = c;
        self
    }
    pub fn border(mut self, c: Color32) -> Self {
        self.border = c;
        self
    }
    pub fn tone(mut self, c: Color32) -> Self {
        self.kind = BtnKind::Cta(c);
        self
    }
    pub fn with_icon(mut self, d: &'static str, ink: Ink, size: f32) -> Self {
        self.icon = Some((d, ink, size));
        self
    }
    pub fn spin(mut self, on: bool) -> Self {
        self.spin = on;
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn sheen(mut self, on: bool) -> Self {
        self.sheen = on;
        self
    }
    pub fn lead(mut self, w: f32) -> Self {
        self.lead = w;
        self
    }
    pub fn w(mut self, w: f32) -> Self {
        self.w = Some(w);
        self
    }

    pub fn width(&self, ui: &Ui) -> f32 {
        if let Some(w) = self.w {
            return w;
        }
        let mut parts = Vec::new();
        if self.lead > 0.0 {
            parts.push(self.lead);
        }
        if self.spin {
            parts.push(13.0);
        }
        if let Some((_, _, s)) = self.icon {
            parts.push(s);
        }
        if !self.label.is_empty() {
            parts.push(text_w(ui, &self.label, self.font.clone()));
        }
        let gaps = 8.0 * (parts.len().saturating_sub(1)) as f32;
        parts.iter().sum::<f32>() + gaps + self.pad * 2.0
    }

    fn sense(&self) -> Sense {
        if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        }
    }

    /// Place the button in the current layout.
    #[allow(dead_code)]
    pub fn show(&self, ui: &mut Ui) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(self.width(ui), self.h), self.sense());
        self.paint(ui, rect, &resp);
        resp
    }

    /// Place the button at an exact rect.
    pub fn at(&self, ui: &mut Ui, rect: Rect, id: Id) -> Response {
        let resp = ui.interact(rect, id, self.sense());
        self.paint(ui, rect, &resp);
        resp
    }

    /// Place the button with its right edge at `right`, vertically centered on `cy`.
    pub fn at_right(&self, ui: &mut Ui, right: f32, cy: f32, id: Id) -> (Response, Rect) {
        let w = self.width(ui);
        let rect = Rect::from_min_size(pos2(right - w, cy - self.h / 2.0), vec2(w, self.h));
        (self.at(ui, rect, id), rect)
    }

    pub fn at_left(&self, ui: &mut Ui, left: f32, cy: f32, id: Id) -> (Response, Rect) {
        let w = self.width(ui);
        let rect = Rect::from_min_size(pos2(left, cy - self.h / 2.0), vec2(w, self.h));
        (self.at(ui, rect, id), rect)
    }

    fn paint(&self, ui: &Ui, rect: Rect, resp: &Response) {
        if !ui.is_rect_visible(rect.expand(4.0)) {
            return;
        }
        let ctx = ui.ctx();
        let now = ui.input(|i| i.time);
        let hov = resp.hovered() && self.enabled;
        let ht = ctx.animate_bool_with_time(resp.id.with("hv"), hov, 0.2);
        let down = self.enabled && resp.is_pointer_button_down_on();
        if hov {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        let p = ui.painter();
        let a = if self.enabled { 1.0 } else { 0.4 };
        let mut r = rect;
        let fg = match self.kind {
            BtnKind::Cta(tone) => {
                r = r.translate(vec2(0.0, -ht));
                if down {
                    r = Rect::from_center_size(r.center(), r.size() * 0.97);
                }
                fill(p, r, self.radius, brighten(tone, 1.0 + 0.08 * ht).gamma_multiply(a));
                if self.sheen && self.enabled {
                    sheen(p, r, self.radius, now);
                }
                self.color.gamma_multiply(a)
            }
            BtnKind::Ghost => {
                if ht > 0.001 {
                    fill(p, r, self.radius, IB_HOVER.gamma_multiply(ht));
                }
                edge(p, r, self.radius, self.border.gamma_multiply(a));
                mix(self.color, TEXT, ht).gamma_multiply(a)
            }
            BtnKind::Plain => {
                if ht > 0.001 {
                    fill(p, r, self.radius, IB_HOVER.gamma_multiply(ht));
                }
                mix(self.color, TEXT, ht).gamma_multiply(a)
            }
        };
        let gal = (!self.label.is_empty()).then(|| g(ui, &self.label, self.font.clone(), fg));
        let mut parts = Vec::new();
        if self.lead > 0.0 {
            parts.push(self.lead);
        }
        if self.spin {
            parts.push(13.0);
        }
        if let Some((_, _, s)) = self.icon {
            parts.push(s);
        }
        if let Some(gal) = &gal {
            parts.push(gal.size().x);
        }
        let content = parts.iter().sum::<f32>() + 8.0 * parts.len().saturating_sub(1) as f32;
        let cy = r.center().y;
        let mut x = r.center().x - content / 2.0;
        if self.lead > 0.0 {
            x += self.lead + 8.0;
        }
        if self.spin {
            let c = pos2(x + 6.5, cy);
            spin_ring(p, c, 5.75, 1.5, rgba(26, 18, 11, 0.3), CTA_INK, now, 0.8);
            x += 13.0 + 8.0;
        }
        if let Some((d, ink, s)) = self.icon {
            icons::paint(ui, Rect::from_center_size(pos2(x + s / 2.0, cy), vec2(s, s)), d, ink, fg);
            x += s + 8.0;
        }
        if let Some(gal) = gal {
            p.galley(pos2(x, cy - gal.size().y / 2.0), gal, fg);
        }
    }
}

// --------------------------------------------------------------- controls

/// A 40×24 switch.
pub fn switch(ui: &mut Ui, tw: &mut Tweens, rect: Rect, id: Id, on: bool, bg: Color32) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let bgc = tw.c(id.with("bg"), bg, 0.45);
    let x = tw.f(id.with("x"), if on { 16.0 } else { 0.0 }, 0.38, Ease::SPRING_HARD);
    let p = ui.painter();
    fill(p, rect, 12.0, bgc);
    p.circle_filled(pos2(rect.left() + 3.0 + 9.0 + x, rect.center().y), 9.0, KNOB);
    resp
}

/// Segmented control with a sliding indicator. Returns the index clicked.
pub fn segmented(ui: &mut Ui, tw: &mut Tweens, rect: Rect, id: Id, labels: &[&str], cur: usize) -> Option<usize> {
    let p = ui.painter().clone();
    card(&p, rect, 10.0, CARD, CARD_EDGE);
    let n = labels.len().max(1) as f32;
    let step = (rect.width() - 6.0) / n;
    let x = tw.f(id.with("ind"), cur as f32 * step, 0.38, Ease::SPRING_SOFT);
    let ind = Rect::from_min_size(pos2(rect.left() + 3.0 + x, rect.top() + 3.0), vec2(step, rect.height() - 6.0));
    fill(&p, ind, 7.0, SEG_ON);
    let mut out = None;
    for (i, label) in labels.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 3.0 + i as f32 * step, rect.top() + 3.0), vec2(step, rect.height() - 6.0));
        let resp = ui.interact(r, id.with(i), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let on = i == cur;
        let c = tw.c(id.with(("c", i)), if on || resp.hovered() { TEXT } else { TEXT2 }, 0.25);
        let gal = g(ui, label, sans(13.5), c);
        put(&p, &gal, r.center(), Align2::CENTER_CENTER);
        if resp.clicked() {
            out = Some(i);
        }
    }
    out
}

/// A horizontal slider in the style of an accent-colored `<input type=range>`.
pub fn slider(ui: &mut Ui, rect: Rect, id: Id, value: &mut u32, min: u32, max: u32) -> bool {
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 16.0, 4.0));
    let mut changed = false;
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on() {
            let t = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
            let v = (min as f32 + t * (max - min) as f32).round() as u32;
            if v != *value {
                *value = v;
                changed = true;
            }
        }
    }
    let t = (*value - min) as f32 / (max - min).max(1) as f32;
    let p = ui.painter();
    fill(p, track, 2.0, SWITCH_OFF);
    fill(p, Rect::from_min_max(track.min, pos2(track.left() + track.width() * t, track.bottom())), 2.0, COPPER);
    let knob = pos2(track.left() + track.width() * t, track.center().y);
    let hv = ui.ctx().animate_bool_with_time(id.with("hv"), resp.hovered() || resp.dragged(), 0.15);
    p.circle_filled(knob, 8.0 + hv, COPPER);
    p.circle_stroke(knob, 8.0 + hv, Stroke::new(1.0_f32, brighten(COPPER, 0.8)));
    changed
}

// ------------------------------------------------------------- status dot

/// The 10px status glyph used for containers, machines, pods and steps.
/// Returns true when it is animating continuously.
pub fn status_dot(p: &Painter, tw: &mut Tweens, key: Id, c: Pos2, st: St, bad: bool, fresh_at: f64) -> bool {
    let now = tw.now;
    let tint = if bad { COPPER } else { VERDIGRIS };
    let (fill_c, edge_c) = match st {
        St::Running => (tint, tint),
        St::Paused => (Color32::TRANSPARENT, VERDIGRIS),
        St::Error => (DANGER, DANGER),
        St::Starting => (COPPER, COPPER),
        St::Stopping => (Color32::TRANSPARENT, MUTED),
        St::Stopped => (Color32::TRANSPARENT, DIM),
    };
    let fill_c = tw.c(key.with("f"), fill_c, 0.7);
    let edge_c = tw.c(key.with("e"), edge_c, 0.7);
    let mut ambient = false;

    // patina pulse when something has just come up
    let ft = ((now - fresh_at) / 1.2) as f32;
    if (0.0..1.0).contains(&ft) {
        let e = Ease::EASE_OUT.at(ft);
        let col = mix(rgba(212, 145, 92, 0.85), rgba(111, 194, 176, 0.0), e);
        p.circle_filled(c, 5.0 + 16.0 * e, col);
        tw.busy = true;
    }
    match st {
        St::Running => {
            let period = 2.8;
            let ph = ((now % period) / period) as f32;
            if ph < 0.7 {
                let e = Ease::EASE_OUT.at(ph / 0.7);
                p.circle_filled(c, 5.0 * (1.0 + 1.6 * e), tint.gamma_multiply(0.5 * (1.0 - e)));
            }
            ambient = true;
        }
        St::Starting => {
            spin_ring(p, c, 9.25, 1.5, rgba(212, 145, 92, 0.22), COPPER, now, 0.8);
            ambient = true;
        }
        St::Stopping => {
            spin_ring(p, c, 9.25, 1.5, RING_TRACK, MUTED, now, 0.8);
            ambient = true;
        }
        _ => {}
    }
    p.circle_filled(c, 5.0, fill_c);
    p.circle_stroke(c, 4.25, Stroke::new(1.5_f32, edge_c));
    ambient
}

/// A plain 7px dot.
pub fn small_dot(p: &Painter, c: Pos2, color: Color32) {
    p.circle_filled(c, 3.5, color);
}

/// Blinking block cursor (`@keyframes blink`, 1.1s steps).
pub fn cursor_block(p: &Painter, r: Rect, now: f64) {
    if (now % 1.1) < 0.55 {
        p.rect_filled(r, 0.0, TEXT);
    }
}

/// The Patina mark: a copper disc with its verdigris half.
pub fn logo(p: &Painter, c: Pos2, r: f32) {
    p.circle_filled(c, r, COPPER);
    pie(p, c, r, -PI / 4.0, PI, VERDIGRIS);
}
