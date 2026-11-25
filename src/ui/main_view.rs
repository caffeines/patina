//! The main column: banner, toolbar, header, and whichever view is active.

use egui::text::{LayoutJob, TextFormat};
use egui::{pos2, vec2, Align, Align2, Context, CursorIcon, Frame, Id, Margin, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, UiBuilder, ViewportCommand};

use super::{assets, containers, enter, kube, machines, settings, swap, Cx};
use crate::icons::{self, Ink};
use crate::sim::{fmt_count, fmt_size, Banner, Engine, Filter, Job, ModalKind, Pane, St, State, View};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

pub fn show(ctx: &Context, cx: &mut Cx) {
    egui::CentralPanel::default().frame(Frame::new().fill(BG)).show(ctx, |ui| {
        let full = ui.max_rect();
        let d = ui.interact(Rect::from_min_size(full.min, vec2(full.width(), 12.0)), Id::new("main-drag"), Sense::drag());
        if d.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let view = cx.s.view;
        ScrollArea::vertical()
            .id_salt(("main-scroll", view.slug()))
            .auto_shrink([false, false])
            .drag_to_scroll(false)
            .show(ui, |ui| {
                Frame::new().inner_margin(Margin { left: 28, right: 28, top: 12, bottom: 28 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    banner(ui, cx);
                    let (a, dy) = swap(cx.tw, cx.s.view_at, 0.45);
                    ui.multiply_opacity(a);
                    ui.add_space(dy);
                    toolbar(ui, cx);
                    ui.add_space(22.0);
                    header(ui, cx);
                    ui.add_space(22.0);
                    content(ui, cx);
                });
            });
    });
}

// ------------------------------------------------------------------ banner

fn banner(ui: &mut Ui, cx: &mut Cx) {
    let Some(b) = cx.s.banner else { return };
    let spare_mb: f64 = cx.s.spare_images().iter().map(|a| a.mb).sum();
    let (c, bd, title, body, yes) = match b {
        Banner::Update => (COPPER, WARN_EDGE, "Patina 1.4 is ready.".to_string(), "It installs the next time the engine restarts.".to_string(), "Restart and update"),
        Banner::Disk => (
            DANGER,
            ERR_EDGE,
            "The disk image is 92% full.".to_string(),
            format!("58.9 of 64 GB used. Unused images take up {}.", fmt_size(spare_mb)),
            "Prune unused",
        ),
    };
    let wd = ui.available_width();
    let yes_btn = Btn::cta(yes).h(30.0).pad(13.0).radius(8.0).tone(c);
    let yes_w = yes_btn.width(ui);
    let text_w = wd - 16.0 - 7.0 - 12.0 - 12.0 - yes_w - 12.0 - 30.0 - 8.0;
    let mut job = LayoutJob::default();
    job.append(&title, 0.0, TextFormat { font_id: medium(13.5), color: TEXT, ..Default::default() });
    job.append(&body, 5.0, TextFormat { font_id: sans(13.5), color: TEXT2, ..Default::default() });
    job.wrap.max_width = text_w.max(80.0);
    let gal = ui.fonts(|f| f.layout_job(job));
    let h = gal.size().y.max(30.0) + 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(wd, h), Sense::hover());
    let (a, dy) = enter(cx.tw, cx.started, 0.0, 0.5);
    let rect = rect.translate(vec2(0.0, dy));
    let mut bui = ui.new_child(UiBuilder::new().max_rect(rect));
    bui.multiply_opacity(a);
    let p = bui.painter().clone();
    w::card(&p, rect, 12.0, BANNER, bd);
    let cy = rect.center().y;
    w::small_dot(&p, pos2(rect.left() + 16.0 + 3.5, cy), c);
    w::put(&p, &gal, pos2(rect.left() + 16.0 + 7.0 + 12.0, cy), Align2::LEFT_CENTER);
    let close = Btn::icon(icons::CLOSE, Ink::Stroke(1.4)).color(MUTED);
    let (cr, _) = close.at_right(&mut bui, rect.right() - 8.0, cy, Id::new("banner-x"));
    if cr.clicked() {
        cx.fx.push(Box::new(|s| s.banner = None));
    }
    let (yr, _) = yes_btn.at_right(&mut bui, rect.right() - 8.0 - 30.0 - 12.0, cy, Id::new("banner-yes"));
    if yr.clicked() {
        cx.fx.push(match b {
            Banner::Update => Box::new(|s: &mut State| s.apply_update()),
            Banner::Disk => Box::new(|s: &mut State| s.ask_modal(ModalKind::Prune, "")),
        });
    }
    ui.add_space(14.0);
}

// ----------------------------------------------------------------- toolbar

fn chrome(view: View) -> (&'static str, Option<&'static str>) {
    match view {
        View::Containers => ("Containers", Some("Run image")),
        View::Images => ("Images", Some("Pull image")),
        View::Volumes => ("Volumes", Some("Create volume")),
        View::Networks => ("Networks", Some("Create network")),
        View::Machines => ("Machines", Some("New machine")),
        View::Kubernetes => ("Kubernetes", None),
        View::Settings => ("Settings", None),
    }
}

fn is_asset(view: View) -> bool {
    matches!(view, View::Images | View::Volumes | View::Networks)
}

fn toolbar(ui: &mut Ui, cx: &mut Cx) {
    let wd = ui.available_width();
    let (row, bg) = ui.allocate_exact_size(vec2(wd, 34.0), Sense::click_and_drag());
    if bg.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    let view = cx.s.view;
    let live = cx.s.live();
    let down = matches!(cx.s.engine, Engine::Off | Engine::Failed);
    let cy = row.center().y;
    let mut right = row.right();

    // Primary action
    if let (_, Some(label)) = chrome(view) {
        let off = !live || (view == View::Images && (cx.s.pull_busy() || cx.s.pulled));
        let btn = Btn::cta(label).pad(14.0).with_icon(icons::PLUS, Ink::Stroke(1.8), 16.0).sheen(true).enabled(!off);
        let (resp, r) = btn.at_right(ui, right, cy, Id::new(("act", view.slug())));
        if resp.clicked() {
            cx.fx.push(match view {
                View::Containers => Box::new(|s: &mut State| s.open_run("")),
                View::Images => Box::new(|s: &mut State| s.pull_image()),
                View::Volumes => Box::new(|s: &mut State| s.ask_modal(ModalKind::Volume, "")),
                View::Networks => Box::new(|s: &mut State| s.ask_modal(ModalKind::Network, "")),
                _ => Box::new(|s: &mut State| s.open_mk()),
            });
        }
        right = r.left() - 12.0;
    }
    // Secondary: prune
    if view == View::Images && live && !cx.s.spare_images().is_empty() {
        let (resp, _) = Btn::ghost("Prune unused").at_right(ui, right, cy, Id::new("prune"));
        if resp.clicked() {
            cx.fx.push(Box::new(|s| s.ask_modal(ModalKind::Prune, "")));
        }
    }

    // Search
    if (view == View::Containers || is_asset(view)) && !down {
        let id = Id::new("search");
        let focused = ui.memory(|m| m.has_focus(id));
        let sw = cx.tw.f("search-w", if focused { 340.0 } else { 260.0 }, 0.35, Ease::OUT);
        let box_r = Rect::from_min_size(pos2(row.left(), row.top()), vec2(sw, 34.0));
        let p = ui.painter().clone();
        let bc = cx.tw.c("search-b", if focused { SEARCH_FOCUS } else { NAV_ON_EDGE }, 0.25);
        w::fill(&p, box_r, 9.0, SEARCH);
        w::edge(&p, box_r, 9.0, bc);
        icons::paint(ui, Rect::from_center_size(pos2(box_r.left() + 10.0 + 8.0, cy), vec2(16.0, 16.0)), icons::SEARCH, Ink::Stroke(1.4), MUTED);
        // ⌘K chip
        let chip_label = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl K" };
        let chip_w = w::text_w(ui, chip_label, mono(11.0)) + 12.0;
        let chip = Rect::from_min_size(pos2(box_r.right() - 6.0 - chip_w, cy - 11.0), vec2(chip_w, 22.0));
        let chip_resp = ui.interact(chip, Id::new("search-chip"), Sense::click());
        let hv = ui.ctx().animate_bool_with_time(chip_resp.id.with("hv"), chip_resp.hovered(), 0.2);
        if chip_resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if hv > 0.0 {
            w::fill(&p, chip, 5.0, IB_HOVER.gamma_multiply(hv));
        }
        w::edge(&p, chip, 5.0, NAV_ON_EDGE);
        w::text(&p, ui, chip.left() + 6.0, cy, chip_label, mono(11.0), mix(MUTED, TEXT, hv));
        if chip_resp.clicked() {
            cx.fx.push(Box::new(|s| s.open_pal()));
        }
        let input = Rect::from_min_max(pos2(box_r.left() + 34.0, row.top() + 1.0), pos2(chip.left() - 6.0, row.bottom() - 1.0));
        let te = TextEdit::singleline(&mut cx.s.q)
            .id(id)
            .frame(false)
            .font(sans(13.5))
            .text_color(TEXT)
            .hint_text(RichText::new(format!("Search {}", view.slug())).color(MUTED).font(sans(13.5)))
            .desired_width(input.width())
            .min_size(input.size())
            .vertical_align(Align::Center)
            .margin(Margin::ZERO);
        ui.put(input, te);
    }
}

// ------------------------------------------------------------------ header

fn summary(s: &State, view: View) -> String {
    let down = matches!(s.engine, Engine::Off | Engine::Failed);
    let is_set = view == View::Settings;
    if !is_set && s.engine == Engine::Off {
        return "The engine is stopped.".into();
    }
    if !is_set && s.engine == Engine::Failed {
        return "The engine could not start.".into();
    }
    let _ = down;
    if s.engine == Engine::Booting && s.cold && !is_set {
        return "Starting the engine".into();
    }
    let tally = |st: St| s.alive().filter(|c| c.st == st).count();
    match view {
        View::Containers => {
            let mut parts = vec![
                format!("{} running", tally(St::Running)),
                format!("{} stopped", tally(St::Stopped) + tally(St::Stopping) + tally(St::Error)),
            ];
            if tally(St::Paused) > 0 {
                parts.push(format!("{} paused", tally(St::Paused)));
            }
            if tally(St::Starting) > 0 {
                parts.push(format!("{} starting", tally(St::Starting)));
            }
            parts.join(" · ")
        }
        View::Images | View::Volumes => {
            let list = if view == View::Images { &s.imgs } else { &s.vols };
            let here: Vec<_> = list.iter().filter(|a| !a.gone).collect();
            let total: f64 = here.iter().map(|a| a.mb).sum();
            let word = if view == View::Images { "image" } else { "volume" };
            format!("{} · {} on disk", fmt_count(here.len(), word), fmt_size(total))
        }
        View::Networks => {
            let n = s.nets.iter().filter(|a| !a.gone).count();
            format!("{} · {} attached", fmt_count(n, "network"), fmt_count(s.alive().count(), "container"))
        }
        View::Machines => "Linux machines that share one kernel with your containers.".into(),
        View::Kubernetes => "A single-node cluster for local development.".into(),
        View::Settings => "Resource limits apply after the engine restarts.".into(),
    }
}

fn header(ui: &mut Ui, cx: &mut Cx) {
    let view = cx.s.view;
    let wd = ui.available_width();
    let is_set = view == View::Settings;
    let down = matches!(cx.s.engine, Engine::Off | Engine::Failed);
    let cold = cx.s.engine == Engine::Booting && cx.s.cold && !is_set;
    let seg: Vec<&str> = match view {
        View::Containers => vec!["All", "Running", "Stopped"],
        View::Settings => Pane::ALL.iter().map(|p| p.label()).collect(),
        _ => vec![],
    };
    let has_seg = !seg.is_empty() && (is_set || (!down && !cold && cx.s.alive().count() > 0));
    let step = if view == View::Containers { 82.0 } else { 88.0 };
    let seg_w = seg.len() as f32 * step + 6.0;

    let title = w::g_ls(ui, chrome(view).0, semibold(28.0), TEXT, -0.7);
    let text_max = if has_seg { wd - seg_w - 16.0 } else { wd };
    let sum = w::g_wrap(ui, &summary(cx.s, view), sans(13.5), TEXT2, text_max, 19.0);
    let h = (32.0 + 2.0 + sum.size().y).max(34.0);
    let (rect, _) = ui.allocate_exact_size(vec2(wd, h), Sense::hover());
    let p = ui.painter().clone();
    w::put(&p, &title, pos2(rect.left(), rect.top() + 16.0), Align2::LEFT_CENTER);
    w::put(&p, &sum, pos2(rect.left(), rect.top() + 34.0), Align2::LEFT_TOP);

    if has_seg {
        let sr = Rect::from_min_size(pos2(rect.right() - seg_w, rect.bottom() - 34.0), vec2(seg_w, 34.0));
        let cur = if is_set {
            Pane::ALL.iter().position(|p| *p == cx.s.pane).unwrap_or(0)
        } else {
            match cx.s.filter {
                Filter::All => 0,
                Filter::Running => 1,
                Filter::Stopped => 2,
            }
        };
        if let Some(i) = w::segmented(ui, cx.tw, sr, Id::new(("seg", view.slug())), &seg, cur) {
            if is_set {
                let pane = Pane::ALL[i];
                cx.fx.push(Box::new(move |s| s.set_pane(pane)));
            } else {
                let f = [Filter::All, Filter::Running, Filter::Stopped][i];
                cx.fx.push(Box::new(move |s| s.filter = f));
            }
        }
    }
}

// ----------------------------------------------------------------- content

struct Block {
    d: &'static str,
    ring: egui::Color32,
    ink: egui::Color32,
    title: String,
    body: String,
    yes: String,
    go: Job,
    no: Option<(String, Job)>,
    log: bool,
}

fn block(d: &'static str, title: impl Into<String>, body: impl Into<String>, yes: &str, go: Job) -> Block {
    Block { d, ring: DASH, ink: MUTED, title: title.into(), body: body.into(), yes: yes.into(), go, no: None, log: false }
}

fn blocking(s: &State) -> Option<Block> {
    let view = s.view;
    let is_set = view == View::Settings;
    let cold = s.engine == Engine::Booting && s.cold && !is_set;
    let q = s.q.trim().to_lowercase();
    if !is_set && s.engine == Engine::Off {
        return Some(block(
            icons::PLUG,
            "The engine is stopped",
            "Containers, images, volumes and machines are unavailable until it starts.",
            "Start engine",
            Box::new(|s| s.cold_boot()),
        ));
    }
    if !is_set && s.engine == Engine::Failed {
        let mut b = block(
            icons::PLUG,
            "The engine could not start",
            "macOS refused to create the virtual machine because too little memory is free. Close a few apps, or lower the memory limit in Settings.",
            "Try again",
            Box::new(|s| s.cold_boot()),
        );
        b.ring = ERR_EDGE;
        b.ink = DANGER;
        b.no = Some((if s.show_log { "Hide log" } else { "Show log" }.into(), Box::new(|s| s.show_log = !s.show_log)));
        b.log = s.show_log;
        return Some(b);
    }
    if cold {
        return None;
    }
    let boxes: Vec<_> = s.alive().collect();
    if view == View::Containers {
        if boxes.is_empty() {
            return Some(block(
                icons::CONTAINERS,
                "No containers yet",
                "Run an image to create your first container. Anything you start with docker or Compose shows up here too.",
                "Run image",
                Box::new(|s| s.open_run("")),
            ));
        }
        let shown = boxes.iter().filter(|c| containers::matches(c, s.filter, &q)).count();
        if shown == 0 {
            let all = boxes.len();
            let filter = match s.filter {
                Filter::All => "all",
                Filter::Running => "running",
                Filter::Stopped => "stopped",
            };
            return Some(if !q.is_empty() {
                block(
                    icons::SEARCH,
                    format!("No containers match “{}”", s.q.trim()),
                    format!("Check the spelling, or clear the search to see all {all}."),
                    "Clear search",
                    Box::new(|s| {
                        s.q.clear();
                        s.filter = Filter::All;
                    }),
                )
            } else {
                block(
                    icons::SEARCH,
                    format!("No {filter} containers"),
                    format!("Switch the filter to see all {all}."),
                    "Show all",
                    Box::new(|s| s.filter = Filter::All),
                )
            });
        }
    }
    if is_asset(view) {
        let names: Vec<String> = match view {
            View::Images => s.imgs.iter().filter(|a| !a.gone).map(|a| a.n.clone()).collect(),
            View::Volumes => s.vols.iter().filter(|a| !a.gone).map(|a| a.n.clone()).collect(),
            _ => s.nets.iter().filter(|a| !a.gone).map(|a| a.n.clone()).collect(),
        };
        if names.is_empty() && s.pull.is_none() {
            return Some(if view == View::Volumes {
                block(
                    icons::VOLUMES,
                    "No volumes yet",
                    "A volume keeps data when its container is deleted. Create one here, or let Compose create them for you.",
                    "Create volume",
                    Box::new(|s| s.ask_modal(ModalKind::Volume, "")),
                )
            } else {
                block(
                    icons::IMAGES,
                    "No images yet",
                    "Pull an image from a registry. Images you build with docker build appear here too.",
                    "Pull image",
                    Box::new(|s| s.pull_image()),
                )
            });
        }
        if !names.is_empty() && !names.iter().any(|n| q.is_empty() || n.to_lowercase().contains(&q)) {
            let word = match view {
                View::Images => "image",
                View::Volumes => "volume",
                _ => "network",
            };
            return Some(block(
                icons::SEARCH,
                format!("No {word}s match “{}”", s.q.trim()),
                format!("Check the spelling, or clear the search to see all {}.", names.len()),
                "Clear search",
                Box::new(|s| s.q.clear()),
            ));
        }
    }
    None
}

fn content(ui: &mut Ui, cx: &mut Cx) {
    let view = cx.s.view;
    let is_set = view == View::Settings;
    let cold = cx.s.engine == Engine::Booting && cx.s.cold && !is_set;
    if let Some(b) = blocking(cx.s) {
        show_block(ui, cx, b);
        return;
    }
    if cold {
        skeleton(ui, cx);
        return;
    }
    match view {
        View::Containers => containers::show(ui, cx),
        View::Images | View::Volumes | View::Networks => assets::show(ui, cx),
        View::Machines => machines::show(ui, cx),
        View::Kubernetes => kube::show(ui, cx),
        View::Settings => settings::show(ui, cx),
    }
}

fn show_block(ui: &mut Ui, cx: &mut Cx, b: Block) {
    let wd = ui.available_width();
    let body = {
        let mut j = w::job(&b.body, sans(13.5), TEXT2, 0.0);
        j.sections[0].format.line_height = Some(19.0);
        j.wrap.max_width = 340.0f32.min(wd);
        j.halign = Align::Center;
        ui.fonts(|f| f.layout_job(j))
    };
    let title = w::g_ls(ui, &b.title, semibold(19.0), TEXT, -0.285);
    let log = b.log.then(|| w::g_wrap(ui, State::boot_log(), mono(11.5), CODE, 560.0f32.min(wd) - 32.0, 19.5));
    let content_h = 72.0 + 6.0 + 12.0 + title.size().y + 12.0 + body.size().y + 12.0 + 8.0 + 34.0 + log.as_ref().map_or(0.0, |g| 12.0 + 10.0 + g.size().y + 28.0);
    let h = content_h.max(440.0) + 48.0;
    let (rect, _) = ui.allocate_exact_size(vec2(wd, h), Sense::hover());
    let (a, dy) = enter(cx.tw, cx.s.view_at, 0.1, 0.5);
    let mut bui = ui.new_child(UiBuilder::new().max_rect(rect));
    bui.multiply_opacity(a);
    let p = bui.painter().clone();
    let cx_ = rect.center().x;
    let mut y = rect.top() + (h - content_h) / 2.0 + dy;

    // Slowly turning dashed ring around the icon.
    let c = pos2(cx_, y + 36.0);
    let rot = ((cx.now % 22.0) / 22.0) as f32 * std::f32::consts::TAU;
    cx.ambient = true;
    w::dashed_circle(&p, c, 35.25, Stroke::new(1.5_f32, b.ring), 4.5, 4.0, rot);
    icons::paint(&bui, Rect::from_center_size(c, vec2(28.0, 28.0)), b.d, Ink::Stroke(1.1), b.ink);
    y += 72.0 + 6.0 + 12.0;
    w::put(&p, &title, pos2(cx_, y), Align2::CENTER_TOP);
    y += title.size().y + 12.0;
    // body is laid out centered around x = 0
    p.galley(pos2(cx_, y), body.clone(), TEXT2);
    y += body.size().y + 12.0 + 8.0;

    let yes = Btn::cta(b.yes.clone());
    let yes_w = yes.width(&bui);
    let no = b.no.as_ref().map(|(label, _)| Btn::ghost(label.clone()));
    let no_w = no.as_ref().map_or(0.0, |n| n.width(&bui) + 10.0);
    let x = cx_ - (yes_w + no_w) / 2.0;
    if yes.at(&mut bui, Rect::from_min_size(pos2(x, y), vec2(yes_w, 34.0)), Id::new("block-yes")).clicked() {
        cx.fx.push(b.go);
    }
    if let (Some(no), Some((_, go))) = (no, b.no) {
        if no.at(&mut bui, Rect::from_min_size(pos2(x + yes_w + 10.0, y), vec2(no_w - 10.0, 34.0)), Id::new("block-no")).clicked() {
            cx.fx.push(go);
        }
    }
    y += 34.0;
    if let Some(log) = log {
        y += 10.0;
        let lw = 560.0f32.min(wd);
        let lr = Rect::from_min_size(pos2(cx_ - lw / 2.0, y), vec2(lw, log.size().y + 28.0));
        w::fill(&p, lr, 10.0, TERM);
        p.galley(pos2(lr.left() + 16.0, lr.top() + 14.0), log, CODE);
    }
}

fn skeleton(ui: &mut Ui, cx: &mut Cx) {
    let wd = ui.available_width();
    let rows = [(38.0, 54.0), (30.0, 46.0), (44.0, 60.0), (26.0, 40.0), (36.0, 52.0), (32.0, 48.0), (40.0, 58.0)];
    let (rect, _) = ui.allocate_exact_size(vec2(wd, rows.len() as f32 * 54.0), Sense::hover());
    let p = ui.painter().clone();
    cx.ambient = true;
    let now = cx.now;
    for (i, (a, b)) in rows.iter().enumerate() {
        let top = rect.top() + i as f32 * 54.0;
        let cy = top + 27.0;
        let x = rect.left() + 14.0;
        w::skeleton(&p, Rect::from_center_size(pos2(x + 5.0, cy), vec2(10.0, 10.0)), 5.0, now);
        let flex_l = x + 10.0 + 16.0;
        let flex_r = rect.right() - 14.0 - 52.0 - 16.0 - 46.0 - 16.0;
        let fw = flex_r - flex_l;
        w::skeleton(&p, Rect::from_min_size(pos2(flex_l, cy - 13.0), vec2(fw * a / 100.0, 10.0)), 5.0, now);
        w::skeleton(&p, Rect::from_min_size(pos2(flex_l, cy + 5.0), vec2(fw * b / 100.0, 8.0)), 4.0, now);
        w::skeleton(&p, Rect::from_min_size(pos2(flex_r + 16.0, cy - 7.0), vec2(46.0, 14.0)), 5.0, now);
        w::skeleton(&p, Rect::from_min_size(pos2(rect.right() - 14.0 - 52.0, cy - 5.0), vec2(52.0, 10.0)), 5.0, now);
    }
}
