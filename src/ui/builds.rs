//! Builds: the engine's build history, with a detail panel and the way into
//! each build's log.

use egui::{pos2, vec2, Align2, CursorIcon, Id, Pos2, Rect, Sense, Stroke, Ui, UiBuilder};

use super::{enter, swap, Cx};
use crate::live::ago;
use crate::sim::{Build, BuildSt, St};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

pub fn matches(b: &Build, q: &str) -> bool {
    q.is_empty() || b.name.to_lowercase().contains(q) || b.id.to_lowercase().contains(q)
}

pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Seconds the build took, or has taken so far.
pub fn took(b: &Build, now: i64) -> f64 {
    if b.st == BuildSt::Running {
        (now - b.created).max(0) as f64
    } else {
        b.secs
    }
}

pub fn fmt_secs(s: f64) -> String {
    if s < 10.0 {
        format!("{s:.1}s")
    } else if s < 60.0 {
        format!("{}s", s.round() as i64)
    } else if s < 3600.0 {
        let s = s.round() as i64;
        format!("{}m {}s", s / 60, s % 60)
    } else {
        let m = (s / 60.0).round() as i64;
        format!("{}h {}m", m / 60, m % 60)
    }
}

/// The container status dot that reads the same way.
pub fn dot(st: BuildSt) -> St {
    match st {
        BuildSt::Running => St::Starting,
        BuildSt::Done => St::Running,
        BuildSt::Failed => St::Error,
        BuildSt::Canceled => St::Stopped,
    }
}

pub fn word(st: BuildSt) -> &'static str {
    match st {
        BuildSt::Running => "Building",
        BuildSt::Done => "Completed",
        BuildSt::Failed => "Failed",
        BuildSt::Canceled => "Canceled",
    }
}

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let wd = ui.available_width();
    let aside_w = 348.0;
    let gap = 24.0;
    let stacked = wd < 460.0 + gap + aside_w;
    let list_w = if stacked { wd } else { wd - gap - aside_w };
    let origin = ui.cursor().min;
    let list_h = list(ui, cx, origin, list_w);
    let aside_origin = if stacked { origin + vec2(0.0, list_h + 24.0) } else { origin + vec2(list_w + gap, 0.0) };
    let aside_h = detail(ui, cx, aside_origin, if stacked { wd } else { aside_w });
    let total = if stacked { list_h + 24.0 + aside_h } else { list_h.max(aside_h) };
    ui.advance_cursor_after_rect(Rect::from_min_size(origin, vec2(wd, total)));
}

fn list(ui: &mut Ui, cx: &mut Cx, origin: Pos2, wd: f32) -> f32 {
    let q = cx.s.q.trim().to_lowercase();
    let builds: Vec<Build> = cx.s.builds.iter().filter(|b| matches(b, &q)).cloned().collect();
    let sel = cx.s.build().map(|b| b.id.clone()).unwrap_or_default();
    let now = now_unix();
    let view_at = cx.s.view_at;
    let mut y = origin.y;
    for (n, b) in builds.iter().enumerate() {
        let (a, dy) = enter(cx.tw, view_at, 0.2 + n.min(12) as f32 * 0.04, 0.55);
        let mut rui = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(pos2(origin.x, y), vec2(wd, 54.0))));
        rui.multiply_opacity(a);
        let row = Rect::from_min_size(pos2(origin.x, y + 1.0 + dy), vec2(wd, 52.0));
        build_row(&mut rui, cx, b, row, b.id == sel, now);
        y += 54.0;
    }
    y - origin.y
}

fn build_row(ui: &mut Ui, cx: &mut Cx, b: &Build, row: Rect, selected: bool, now: i64) {
    let p = ui.painter().clone();
    let resp = ui.interact(row, Id::new(("build", &b.id)), Sense::click());
    let hv = cx.tw.f(("build-hv", &b.id), if resp.hovered() { 1.0 } else { 0.0 }, 0.2, Ease::EASE);
    if selected {
        w::card(&p, row, 10.0, ROW_SEL, ROW_SEL_EDGE);
    } else if hv > 0.01 {
        w::fill(&p, row, 10.0, ROW_HOVER.gamma_multiply(hv));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if resp.clicked() {
        let id = b.id.clone();
        cx.fx.push(Box::new(move |s| s.pick_build(&id)));
    }
    if resp.double_clicked() {
        let id = b.id.clone();
        cx.fx.push(Box::new(move |s| s.open_build_log(&id)));
    }

    // Columns: 22px dot | name (flex) | took 70 | age 110
    let cy = row.center().y;
    let x = row.left() + 14.0;
    let age_r = row.right() - 14.0;
    let took_r = age_r - 110.0 - 14.0;
    let name_x = x + 22.0 + 14.0;
    let name_w = (took_r - 70.0 - 14.0 - name_x).max(40.0);
    cx.ambient |= w::status_dot(&p, cx.tw, Id::new(("build-dot", &b.id)), pos2(x + 5.0, cy), dot(b.st), false, -10.0);

    let tag = match b.st {
        BuildSt::Failed => Some(("failed".to_string(), DANGER_TEXT, ERR_EDGE)),
        BuildSt::Canceled => Some(("canceled".to_string(), TEXT2, BTN_EDGE)),
        BuildSt::Running => Some((format!("step {} of {}", b.done_steps, b.steps), COPPER_TEXT, WARN_EDGE)),
        BuildSt::Done => None,
    };
    let tag_g = tag.as_ref().map(|(t, c, _)| w::g(ui, t, sans(11.0), *c));
    let tag_w = tag_g.as_ref().map_or(0.0, |g| g.size().x + 14.0);
    let line1 = cy - 8.0;
    let line2 = cy + 9.8;
    let name = if b.name.is_empty() { &b.id } else { &b.name };
    let name_g = w::g_elide(ui, name, medium(14.0), TEXT, name_w - if tag_w > 0.0 { tag_w + 8.0 } else { 0.0 });
    let nr = w::put(&p, &name_g, pos2(name_x, line1), Align2::LEFT_CENTER);
    if let (Some(tg), Some((_, _, edge))) = (tag_g, tag) {
        let tr = Rect::from_min_size(pos2(nr.right() + 8.0, line1 - 8.5), vec2(tag_w, 17.0));
        w::edge(&p, tr, 6.0, edge);
        w::put(&p, &tg, tr.center(), Align2::CENTER_CENTER);
    }
    let short: String = b.id.chars().take(12).collect();
    let sub = if b.steps > 0 { format!("{short} · {} steps · {} cached", b.steps, b.cached) } else { short };
    let sub_g = w::g_elide(ui, &sub, mono(11.5), MUTED, name_w);
    w::put(&p, &sub_g, pos2(name_x, line2), Align2::LEFT_CENTER);

    let t = took(b, now);
    if b.st == BuildSt::Running {
        cx.ambient = true;
    }
    w::text_right(&p, ui, took_r, cy, &if t > 0.0 { fmt_secs(t) } else { "—".into() }, mono(12.0), TEXT);
    w::text_right(&p, ui, age_r, cy, &ago(now - b.created), sans(12.5), TEXT2);
}

fn detail(ui: &mut Ui, cx: &mut Cx, origin: Pos2, wd: f32) -> f32 {
    let Some(b) = cx.s.build().cloned() else { return 0.0 };
    let info = cx.s.build_info.get(&b.id).cloned();
    let (view_at, build_at) = (cx.s.view_at, cx.s.build_at);
    let now = now_unix();
    let iw = wd - 40.0;
    let err = if b.err.is_empty() { info.as_ref().map(|i| i.err.clone()).unwrap_or_default() } else { b.err.clone() };
    let err_g = (b.st == BuildSt::Failed && !err.is_empty()).then(|| w::g_wrap(ui, &err, sans(12.5), TEXT, iw - 31.0, 18.75));
    let name = if b.name.is_empty() { b.id.clone() } else { b.name.clone() };
    let name_g = w::g_elide(ui, &name, semibold(19.0), TEXT, iw);
    let head_h = 48.0;
    let err_h = err_g.as_ref().map_or(0.0, |g| g.size().y + 22.0 + 18.0);
    let stats_h = 62.0;
    let row_h = 30.5;
    let rows_h = 6.0 * row_h;
    let total = 20.0 + head_h + 18.0 + err_h + stats_h + 22.0 + rows_h + 10.0 + 34.0 + 20.0;

    let (a, dy) = enter(cx.tw, view_at, 0.3, 0.6);
    let card = Rect::from_min_size(origin + vec2(0.0, dy), vec2(wd, total));
    let mut aui = ui.new_child(UiBuilder::new().max_rect(card));
    aui.multiply_opacity(a);
    w::card(aui.painter(), card, 14.0, CARD, CARD_EDGE);
    let (pa, pdy) = swap(cx.tw, build_at, 0.4);
    let mut tui = aui.new_child(UiBuilder::new().max_rect(card));
    tui.multiply_opacity(pa);
    let p = tui.painter().clone();
    let x = card.left() + 20.0;
    let right = card.right() - 20.0;
    let mut y = card.top() + 20.0 + pdy;

    // Name and status
    w::put(&p, &name_g, pos2(x, y + 13.0), Align2::LEFT_CENTER);
    let st_c = match b.st {
        BuildSt::Running => COPPER,
        BuildSt::Done => VERDIGRIS,
        BuildSt::Failed => DANGER,
        BuildSt::Canceled => DIM,
    };
    let sy = y + 26.6 + 4.0 + 8.75;
    w::small_dot(&p, pos2(x + 3.5, sy), cx.tw.c("build-st", st_c, 0.7));
    let t = took(&b, now);
    let line = format!("{} · {} · {}", word(b.st), if t > 0.0 { fmt_secs(t) } else { "—".into() }, ago(now - b.created));
    w::text(&p, &tui, x + 15.0, sy, &line, sans(12.5), TEXT2);
    y += head_h + 18.0;

    if let Some(g) = &err_g {
        let r = Rect::from_min_size(pos2(x, y), vec2(iw, g.size().y + 22.0));
        w::card(&p, r, 10.0, ERR_BG, ERR_EDGE);
        w::small_dot(&p, pos2(x + 14.0 + 3.5, y + 11.0 + 9.5), DANGER);
        p.galley(pos2(x + 14.0 + 7.0 + 10.0, y + 11.0), g.clone(), TEXT);
        y += g.size().y + 22.0 + 18.0;
    }

    // Steps and cache hits
    let col_w = (iw - 16.0) / 2.0;
    let steps = b.steps.max(1) as f32;
    let cached_pct = if b.steps > 0 { format!("{}%", (b.cached as f32 * 100.0 / steps).round() as u32) } else { "—".into() };
    let done = if b.steps > 0 { format!("{} of {}", b.done_steps, b.steps) } else { "—".into() };
    let stats = [("Steps", done, b.done_steps as f32 / steps), ("Cached", cached_pct, b.cached as f32 / steps)];
    for (i, (label, val, f)) in stats.into_iter().enumerate() {
        let cx0 = x + i as f32 * (col_w + 16.0);
        w::text(&p, &tui, cx0, y + 8.5, label, sans(12.0), MUTED);
        let vg = w::g_ls(&tui, &val, mono(22.0), TEXT, -0.66);
        w::put(&p, &vg, pos2(cx0, y + 17.0 + 6.0 + 15.0), Align2::LEFT_CENTER);
        let fv = cx.tw.f(("build-bar", i), f, 1.0, Ease::STD);
        w::bar(&p, Rect::from_min_size(pos2(cx0, y + 59.0), vec2(col_w, 3.0)), fv, VERDIGRIS, TRACK);
    }
    y += stats_h + 22.0;
    p.line_segment([pos2(x, y - 11.0), pos2(right, y - 11.0)], Stroke::new(1.0_f32, CARD_EDGE));

    // Where it came from
    let get = |f: fn(&crate::sim::BuildInfo) -> &String| info.as_ref().map(f).filter(|v| !v.is_empty()).cloned();
    let pending = if info.is_some() { "none".to_string() } else { "…".to_string() };
    let revision = get(|i| &i.revision).map(|r| r.chars().take(12).collect::<String>());
    let rows = [
        ("Dockerfile", get(|i| &i.dockerfile)),
        ("Context", get(|i| &i.context)),
        ("Platform", get(|i| &i.platform)),
        ("Repository", get(|i| &i.repo)),
        ("Revision", revision),
        ("Build args", get(|i| &i.args)),
    ];
    let mut ly = y + 8.75;
    for (k, v) in rows {
        w::text(&p, &tui, x, ly, k, sans(12.5), MUTED);
        let g = w::g_elide(&tui, v.as_deref().unwrap_or(&pending), mono(12.0), TEXT, iw - 92.0);
        w::put(&p, &g, pos2(x + 92.0, ly), Align2::LEFT_CENTER);
        ly += row_h;
    }
    y += rows_h + 10.0;

    let open = Btn::ghost("Open build log").h(34.0).radius(9.0).with_icon(crate::icons::EXPAND, crate::icons::Ink::Stroke(1.4), 14.0);
    if open.at(&mut tui, Rect::from_min_size(pos2(x, y), vec2(iw, 34.0)), Id::new("build-log")).clicked() {
        let id = b.id.clone();
        cx.fx.push(Box::new(move |s| s.open_build_log(&id)));
    }
    total
}
