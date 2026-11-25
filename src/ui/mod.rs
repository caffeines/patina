//! The main window, view by view.

pub mod assets;
pub mod builds;
pub mod containers;
pub mod kube;
pub mod logs;
pub mod machines;
pub mod main_view;
pub mod overlays;
pub mod popover;
pub mod settings;
pub mod side;

use egui::{vec2, Align, Id, Margin, Rect, Response, RichText, TextEdit, Ui};

use crate::sim::{Job, State};
use crate::theme::*;
use crate::tween::{rise, Tweens};
use crate::widgets::{edge, fill};

/// Everything a view needs for one frame. Clicks are queued in `fx` and
/// applied to the state after the frame is drawn.
pub struct Cx<'a> {
    pub s: &'a mut State,
    pub tw: &'a mut Tweens,
    pub fx: Vec<Job>,
    pub now: f64,
    /// When the window first appeared (for entrance animations).
    pub started: f64,
    /// Something on screen animates continuously (spinners, breathing dots).
    pub ambient: bool,
}

/// Entrance animation measured from `t0`. Returns `(alpha, dy)`.
pub fn enter(tw: &mut Tweens, t0: f64, delay: f32, dur: f32) -> (f32, f32) {
    let (a, dy, running) = rise(tw.now, t0, delay, dur, 10.0);
    if running {
        tw.busy = true;
    }
    (a, dy)
}

/// The view-swap animation (`swapA`/`swapB`): fade in, rise 8px.
pub fn swap(tw: &mut Tweens, t0: f64, dur: f32) -> (f32, f32) {
    let (a, dy, running) = rise(tw.now, t0, 0.0, dur, 8.0);
    if running {
        tw.busy = true;
    }
    (a, dy)
}

/// A single-line text field styled like the design's `.fld` inputs.
#[allow(clippy::too_many_arguments)]
pub fn field(ui: &mut Ui, tw: &mut Tweens, rect: Rect, id: Id, text: &mut String, hint: &str, err: bool, font: egui::FontId) -> Response {
    let focused = ui.memory(|m| m.has_focus(id));
    let bc = tw.c(id.with("border"), if focused { COPPER } else if err { FIELD_ERR } else { BTN_EDGE }, 0.2);
    let p = ui.painter();
    fill(p, rect, 9.0, FIELD);
    edge(p, rect, 9.0, bc);
    let inner = rect.shrink2(vec2(12.0, 1.0));
    let te = TextEdit::singleline(text)
        .id(id)
        .frame(false)
        .font(font.clone())
        .text_color(TEXT)
        .hint_text(RichText::new(hint).color(MUTED).font(font))
        .desired_width(inner.width())
        .min_size(inner.size())
        .vertical_align(Align::Center)
        .margin(Margin::ZERO);
    ui.put(inner, te)
}

/// Focus a field if the state asked for it.
pub fn maybe_focus(s: &mut State, key: &'static str, resp: &Response) {
    if s.focus == Some(key) {
        resp.request_focus();
        s.focus = None;
    }
}
