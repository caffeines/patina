//! The log page: a container's logs or a build's log across the whole main
//! column, following new lines like `tail -f`, with a filter.
//!
//! Lines are drawn in a monospace font and wrap at a fixed number of
//! columns, so each line's height is known without laying it out. That keeps
//! scrolling through tens of thousands of lines cheap: only the visible ones
//! are drawn.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{pos2, vec2, Align2, Color32, Id, Key, Modifiers, Rect, ScrollArea, Ui, UiBuilder};

use super::{builds, field, swap, Cx};
use crate::icons::{self, Ink};
use crate::sim::{LogLine, Page, St, State, TAILS};
use crate::theme::*;
use crate::widgets::{self as w, Btn};

const LH: f32 = 18.0;
const SIZE: f32 = 12.0;
const FILTER: &str = "page-q";

/// Where each shown line starts, kept between frames. It's extended as
/// lines arrive, and rebuilt when they're replaced or the width or filter
/// changes.
#[derive(Clone, Default)]
struct Rows {
    rev: u64,
    len: usize,
    cols: usize,
    q: String,
    /// Indexes into the page's lines that pass the filter.
    shown: Vec<usize>,
    /// `tops[k]` is where `shown[k]` starts; one extra entry for the end.
    tops: Vec<f32>,
}

fn width(m: &str) -> usize {
    m.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

fn passes(l: &LogLine, q: &str) -> bool {
    q.is_empty() || l.m.to_ascii_lowercase().contains(q) || l.t.contains(q)
}

impl Rows {
    fn update(&mut self, lines: &[LogLine], rev: u64, cols: usize, q: &str) {
        if self.rev != rev || self.cols != cols || self.q != q || self.len > lines.len() {
            *self = Rows { rev, cols, q: q.to_string(), tops: vec![0.0], ..Default::default() };
        }
        let mut top = *self.tops.last().unwrap_or(&0.0);
        for (i, l) in lines.iter().enumerate().skip(self.len) {
            if passes(l, q) {
                let rows = width(&l.m).div_ceil(cols).max(1);
                top += rows as f32 * LH;
                self.shown.push(i);
                self.tops.push(top);
            }
        }
        self.len = lines.len();
    }

    fn height(&self) -> f32 {
        *self.tops.last().unwrap_or(&0.0)
    }
}

/// One line's message, wrapped at exactly `cols` columns, with filter
/// matches highlighted.
fn message(m: &str, q: &str, col: Color32, cols: usize, cw: f32) -> LayoutJob {
    let text = m.replace('\t', "    ");
    let mut j = LayoutJob { wrap: TextWrapping { max_width: cols as f32 * cw + 0.5, break_anywhere: true, ..Default::default() }, ..Default::default() };
    let plain = TextFormat { font_id: mono(SIZE), color: col, line_height: Some(LH), ..Default::default() };
    if q.is_empty() {
        j.append(&text, 0.0, plain);
        return j;
    }
    // ASCII lowercasing keeps byte offsets, so matches index `text` too.
    let lower = text.to_ascii_lowercase();
    let mut at = 0;
    for (i, _) in lower.match_indices(q) {
        if i < at {
            continue;
        }
        j.append(&text[at..i], 0.0, plain.clone());
        j.append(&text[i..i + q.len()], 0.0, TextFormat { font_id: mono(SIZE), color: TEXT, background: COPPER.gamma_multiply(0.38), line_height: Some(LH), ..Default::default() });
        at = i + q.len();
    }
    j.append(&text[at..], 0.0, plain);
    j
}

fn tint(k: u8) -> Color32 {
    match k {
        1 => VERDIGRIS,
        2 => COPPER,
        3 => DANGER_TEXT,
        _ => CODE,
    }
}

/// A pod's container, as Docker names it: `k8s_<container>_<pod>_<namespace>_<uid>_<attempt>`.
fn pod_of(id: &str) -> Option<(&str, &str, &str)> {
    let mut parts = id.strip_prefix("k8s_")?.split('_');
    let (container, pod, ns) = (parts.next()?, parts.next()?, parts.next()?);
    Some((container, pod, ns))
}

/// Title, status line and its dot colour for what the page shows.
fn heading(s: &State, page: &Page) -> (String, String, Color32) {
    match page {
        Page::Logs(id) if s.alive().all(|c| c.id != *id) && pod_of(id).is_some() => {
            let (container, pod, ns) = pod_of(id).unwrap_or_default();
            let found = s.ksnap.as_ref().and_then(|k| k.objs.iter().find(|o| o.kind == "Pod" && o.ns == ns && o.name == pod));
            let (status, col) = match found {
                Some(o) => (
                    o.cols.get(1).cloned().unwrap_or_default(),
                    match o.health {
                        crate::engine::kube::Health::Good => VERDIGRIS,
                        crate::engine::kube::Health::Busy => COPPER,
                        crate::engine::kube::Health::Bad => DANGER,
                        crate::engine::kube::Health::Idle => DIM,
                    },
                ),
                None => ("Pod".to_string(), DIM),
            };
            (pod.to_string(), format!("{status} · {container} in {ns}"), col)
        }
        Page::Logs(id) => {
            let Some(c) = s.alive().find(|c| c.id == *id) else { return (id.clone(), "Removed".into(), DIM) };
            let (st, col) = match c.st {
                St::Running if c.bad => ("Running · unhealthy".to_string(), COPPER),
                St::Running => (format!("Running · up {}", c.up), VERDIGRIS),
                St::Starting => ("Starting".into(), COPPER),
                St::Stopping => ("Stopping".into(), DIM),
                St::Paused => ("Paused".into(), VERDIGRIS),
                St::Error => (format!("Failed · {}", if c.tag.is_empty() { "exited" } else { &c.tag }), DANGER),
                St::Stopped => ("Stopped".into(), DIM),
            };
            (c.id.clone(), st, col)
        }
        Page::BuildLog(id) => {
            let Some(b) = s.builds.iter().find(|b| b.id == *id) else { return (id.clone(), "Build".into(), DIM) };
            let t = builds::took(b, builds::now_unix());
            let col = match b.st {
                crate::sim::BuildSt::Running => COPPER,
                crate::sim::BuildSt::Done => VERDIGRIS,
                crate::sim::BuildSt::Failed => DANGER,
                crate::sim::BuildSt::Canceled => DIM,
            };
            let name = if b.name.is_empty() { b.id.clone() } else { b.name.clone() };
            (name, format!("{} · {}", builds::word(b.st), builds::fmt_secs(t)), col)
        }
    }
}

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let Some(page) = cx.s.page.clone() else { return };
    let ctx = ui.ctx().clone();
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
        ctx.memory_mut(|m| m.request_focus(Id::new(FILTER)));
    }
    let full = ui.max_rect();
    let area = Rect::from_min_max(pos2(full.left() + 28.0, full.top() + 12.0), pos2(full.right() - 28.0, full.bottom() - 18.0));
    let (a, dy) = swap(cx.tw, cx.s.page_at, 0.4);
    let mut pui = ui.new_child(UiBuilder::new().max_rect(area));
    pui.multiply_opacity(a);
    let p = pui.painter().clone();
    let mut y = area.top() + dy;

    // ---- back, history, copy
    let cy = y + 17.0;
    let back = match &page {
        Page::Logs(id) if pod_of(id).is_some() => "Kubernetes",
        Page::Logs(_) => "Containers",
        Page::BuildLog(_) => "Builds",
    };
    let (br, _) = Btn::ghost(back).pad(12.0).with_icon(icons::BACK, Ink::Stroke(1.6), 16.0).at_left(&mut pui, area.left(), cy, Id::new("page-back"));
    if br.clicked() {
        cx.fx.push(Box::new(|s| s.close_page()));
    }
    let (cr, crect) = Btn::ghost("Copy").pad(14.0).at_right(&mut pui, area.right(), cy, Id::new("page-copy"));
    if matches!(page, Page::Logs(_)) {
        let labels: Vec<&str> = TAILS.iter().map(|t| t.0).collect();
        let seg_w = labels.len() as f32 * 82.0 + 6.0;
        let sr = Rect::from_min_size(pos2(crect.left() - 12.0 - seg_w, y), vec2(seg_w, 34.0));
        if let Some(i) = w::segmented(&mut pui, cx.tw, sr, Id::new("page-tail"), &labels, cx.s.page_tail) {
            cx.fx.push(Box::new(move |s| s.set_page_tail(i)));
        }
    }
    y += 34.0 + 22.0;

    // ---- title, status, filter
    let (title, status, st_c) = heading(cx.s, &page);
    let fw = 300.0f32.min(area.width() * 0.4);
    let tg = w::g_elide(&pui, &title, semibold(28.0), TEXT, area.width() - fw - 24.0);
    w::put(&p, &tg, pos2(area.left(), y + 16.0), Align2::LEFT_CENTER);
    let sy = y + 34.0 + 9.5;
    w::small_dot(&p, pos2(area.left() + 3.5, sy), cx.tw.c("page-st", st_c, 0.7));
    w::text(&p, &pui, area.left() + 15.0, sy, &status, sans(13.5), TEXT2);
    let fr = Rect::from_min_size(pos2(area.right() - fw, y + 19.0), vec2(fw, 34.0));
    let fresp = field(&mut pui, cx.tw, fr, Id::new(FILTER), &mut cx.s.page_q, "Filter lines   ⌘F", false, sans(13.5));
    if fresp.changed() {
        cx.s.page_jump = true;
    }
    y += 54.0 + 18.0;

    // ---- the log
    let body = Rect::from_min_max(pos2(area.left(), y), pos2(area.right(), area.bottom() - 26.0));
    w::card(&p, body, 12.0, TERM, CARD_EDGE);
    let inner = body.shrink2(vec2(16.0, 12.0));
    let cw = pui.fonts(|f| f.glyph_width(&mono(SIZE), '0'));
    let lines = &cx.s.page_lines;
    let stamps = lines.iter().any(|l| !l.t.is_empty());
    let time_w = if stamps { 8.0 * cw + 14.0 } else { 0.0 };
    let cols = (((inner.width() - time_w - 12.0) / cw).floor() as usize).max(20);
    let q = cx.s.page_q.trim().to_ascii_lowercase();
    let rows_id = Id::new("page-rows");
    let mut rows: Rows = ctx.data_mut(|d| d.remove_temp(rows_id)).unwrap_or_default();
    rows.update(lines, cx.s.page_rev, cols, &q);
    let total_h = rows.height();

    let mut lui = pui.new_child(UiBuilder::new().max_rect(inner));
    let mut sa = ScrollArea::vertical().id_salt(("page-scroll", format!("{page:?}"))).auto_shrink([false, false]).stick_to_bottom(true).drag_to_scroll(false);
    if cx.s.page_jump {
        sa = sa.vertical_scroll_offset(total_h);
    }
    let out = sa.show_viewport(&mut lui, |ui, vp| {
        let o = ui.min_rect().min;
        ui.set_min_size(vec2(ui.available_width(), total_h));
        let lp = ui.painter();
        let start = rows.tops.partition_point(|&t| t <= vp.min.y).saturating_sub(1);
        for k in start..rows.shown.len() {
            let top = rows.tops[k];
            if top > vp.max.y {
                break;
            }
            let l = &lines[rows.shown[k]];
            let ly = o.y + top;
            if stamps {
                let tg = ui.fonts(|f| f.layout_job(message(&l.t, "", MUTED, 8, cw)));
                lp.galley(pos2(o.x, ly), tg, MUTED);
            }
            let g = ui.fonts(|f| f.layout_job(message(&l.m, &q, tint(l.k), cols, cw)));
            lp.galley(pos2(o.x + time_w, ly), g, CODE);
        }
    });
    let at_end = out.state.offset.y + out.inner_rect.height() >= out.content_size.y - 4.0;
    cx.s.page_jump = false;

    let shown = rows.shown.len();
    let total = lines.len();
    ctx.data_mut(|d| d.insert_temp(rows_id, rows));
    if shown == 0 {
        let note = if total > 0 {
            format!("No lines match “{}”", cx.s.page_q.trim())
        } else if matches!(page, Page::Logs(_)) {
            "Waiting for output".to_string()
        } else {
            "Loading the build log".to_string()
        };
        let g = w::g(&pui, &note, sans(13.5), MUTED);
        w::put(&p, &g, body.center(), Align2::CENTER_CENTER);
    }
    if !at_end && shown > 0 {
        let jump = Btn::ghost("Jump to latest").h(30.0).pad(12.0).radius(8.0).font(medium(12.5)).with_icon(icons::DOWN, Ink::Stroke(1.6), 14.0);
        let (jr, _) = jump.at_right(&mut pui, body.right() - 14.0, body.bottom() - 14.0 - 15.0, Id::new("page-jump"));
        if jr.clicked() {
            cx.s.page_jump = true;
        }
    }

    // ---- counts
    let count = if q.is_empty() { format!("{} lines", fmt_n(total)) } else { format!("{} of {} lines", fmt_n(shown), fmt_n(total)) };
    let state = match &page {
        Page::BuildLog(id) if !cx.s.builds.iter().any(|b| b.id == *id && b.st == crate::sim::BuildSt::Running) => "",
        _ if at_end => " · following",
        _ => " · paused",
    };
    w::text(&p, &pui, area.left() + 2.0, area.bottom() - 9.0, &format!("{count}{state}"), sans(12.0), MUTED);

    if cr.clicked() {
        let lines = &cx.s.page_lines;
        let text: String = lines
            .iter()
            .filter(|l| passes(l, &q))
            .map(|l| if l.t.is_empty() { format!("{}\n", l.m) } else { format!("{}  {}\n", l.t, l.m) })
            .collect();
        ctx.copy_text(text);
        let n = if q.is_empty() { total } else { shown };
        cx.fx.push(Box::new(move |s| s.toast_say(format!("Copied {} lines", fmt_n(n)), "", None)));
    }
}

fn fmt_n(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(m: &str) -> LogLine {
        LogLine { t: String::new(), m: m.into(), k: 0 }
    }

    #[test]
    fn wraps_at_fixed_columns() {
        let lines = vec![line("short"), line(&"x".repeat(25)), line("a\tb"), line(&"y".repeat(20))];
        let mut r = Rows::default();
        r.update(&lines, 1, 10, "");
        assert_eq!(r.shown, [0, 1, 2, 3]);
        assert_eq!(r.tops, [0.0, LH, 4.0 * LH, 5.0 * LH, 7.0 * LH]);
        // New lines extend it; a filter rebuilds it.
        let mut more = lines.clone();
        more.push(line("short again"));
        r.update(&more, 1, 10, "");
        assert_eq!(r.shown.len(), 5);
        r.update(&more, 1, 10, "short");
        assert_eq!(r.shown, [0, 4]);
    }

    #[test]
    fn reads_pod_container_names() {
        assert_eq!(pod_of("k8s_web_web-7c9f6d8b5-x2kqp_shop_3f1c9a2e-7d4b_0"), Some(("web", "web-7c9f6d8b5-x2kqp", "shop")));
        assert_eq!(pod_of("cps-postgres"), None);
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(fmt_n(7), "7");
        assert_eq!(fmt_n(1234), "1,234");
        assert_eq!(fmt_n(50000), "50,000");
    }
}
