//! The dashboard: what's on the engine at a glance.
//!
//! - Disk: everything Docker keeps, split by kind on one bar, with what's
//!   unused, what Clean up could free, and the disk image macOS allocated.
//! - The engine: status, versions, memory, and its switches.
//! - Counts: containers, images, volumes, networks, machines, Kubernetes,
//!   each opening its view.
//! - Activity: CPU and memory of all containers over the last minute, and
//!   the busiest containers now.
//! - The biggest images and volumes, and links to open ports and machines.

use egui::{pos2, vec2, Align2, Color32, CursorIcon, Id, Rect, Sense, Shape, Stroke, Ui, UiBuilder};

use super::{enter, Cx};
use crate::engine::cleanup::{fmt_bytes, Item, Usage};
use crate::icons::{self, Ink};
use crate::sim::{Engine, Kube, St, State, View};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

// The five kinds of disk use, in bar order. Checked as a categorical palette
// against CARD with the dataviz validator: lightness band, chroma floor,
// adjacent colorblind separation (worst ΔE 14.6) and contrast all pass.
const C_IMAGES: Color32 = hex(0xC9834F);
const C_CONTAINERS: Color32 = hex(0x338CED);
const C_MACHINES: Color32 = hex(0x2FAF93);
const C_VOLUMES: Color32 = hex(0x9887EF);
const C_CACHE: Color32 = hex(0xE24D8B);

const GAP: f32 = 20.0;

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    cx.ambient = true;
    let wd = ui.available_width();
    let origin = ui.cursor().min;
    let u = cx.s.disk();
    let mut y = origin.y;

    // Disk and engine
    let wide = wd >= 860.0;
    let top_h = 384.0;
    let disk_w = if wide { ((wd - GAP) * 0.6).floor() } else { wd };
    disk_card(ui, cx, Rect::from_min_size(pos2(origin.x, y), vec2(disk_w, top_h)), u.as_ref());
    let eng = if wide {
        Rect::from_min_size(pos2(origin.x + disk_w + GAP, y), vec2(wd - disk_w - GAP, top_h))
    } else {
        Rect::from_min_size(pos2(origin.x, y + top_h + GAP), vec2(wd, 300.0))
    };
    engine_card(ui, cx, eng);
    y = eng.bottom() + GAP;

    // Counts
    y = tiles(ui, cx, origin.x, y, wd, u.as_ref()) + GAP;

    // Activity
    let act_h = 214.0;
    if wide {
        let w3 = ((wd - 2.0 * GAP) / 3.4).floor();
        spark_card(ui, cx, Rect::from_min_size(pos2(origin.x, y), vec2(w3, act_h)), true, 0.30);
        spark_card(ui, cx, Rect::from_min_size(pos2(origin.x + w3 + GAP, y), vec2(w3, act_h)), false, 0.34);
        busiest(ui, cx, Rect::from_min_max(pos2(origin.x + 2.0 * (w3 + GAP), y), pos2(origin.x + wd, y + act_h)));
        y += act_h + GAP;
    } else {
        let half = ((wd - GAP) / 2.0).floor();
        spark_card(ui, cx, Rect::from_min_size(pos2(origin.x, y), vec2(half, act_h)), true, 0.30);
        spark_card(ui, cx, Rect::from_min_size(pos2(origin.x + half + GAP, y), vec2(wd - half - GAP, act_h)), false, 0.34);
        y += act_h + GAP;
        busiest(ui, cx, Rect::from_min_size(pos2(origin.x, y), vec2(wd, act_h)));
        y += act_h + GAP;
    }

    // Lists
    let list_h = 286.0;
    let cols = if wd >= 1000.0 { 3 } else if wd >= 640.0 { 2 } else { 1 };
    let cw = (wd - GAP * (cols - 1) as f32) / cols as f32;
    let at = |i: usize| Rect::from_min_size(pos2(origin.x + (i % cols) as f32 * (cw + GAP), y + (i / cols) as f32 * (list_h + GAP)), vec2(cw, list_h));
    let none = Vec::new();
    let imgs = u.as_ref().map_or(&none, |u| &u.biggest_images);
    let vols = u.as_ref().map_or(&none, |u| &u.biggest_volumes);
    ranked(ui, cx, at(0), "Biggest images", imgs, C_IMAGES, View::Images, "No images yet.", 0.42);
    ranked(ui, cx, at(1), "Largest volumes", vols, C_VOLUMES, View::Volumes, "No volumes yet.", 0.46);
    links(ui, cx, at(2));
    let rows = 3usize.div_ceil(cols);
    y += rows as f32 * list_h + (rows - 1) as f32 * GAP;

    ui.advance_cursor_after_rect(Rect::from_min_max(origin, pos2(origin.x + wd, y)));
}

/// A card that rises in with the view, and a child ui to draw it in.
fn card(ui: &mut Ui, cx: &mut Cx, r: Rect, delay: f32) -> (Ui, Rect) {
    let (a, dy) = enter(cx.tw, cx.s.view_at, delay, 0.55);
    let r = r.translate(vec2(0.0, dy));
    let mut cui = ui.new_child(UiBuilder::new().max_rect(r));
    cui.multiply_opacity(a);
    w::card(cui.painter(), r, 14.0, CARD, CARD_EDGE);
    (cui, r)
}

fn title(ui: &Ui, r: Rect, text: &str) {
    w::text(ui.painter(), ui, r.left() + 22.0, r.top() + 20.0 + 9.0, text, medium(13.5), TEXT2);
}

/// A row that highlights under the pointer and opens `view` when clicked.
fn hover_row(ui: &mut Ui, cx: &mut Cx, r: Rect, id: Id, view: Option<View>) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    let hv = cx.tw.f(id.with("hv"), if resp.hovered() { 1.0 } else { 0.0 }, 0.2, Ease::EASE);
    if hv > 0.01 {
        w::fill(ui.painter(), r, 8.0, ROW_HOVER.gamma_multiply(hv * 1.6));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if resp.clicked() {
        if let Some(v) = view {
            cx.fx.push(Box::new(move |s| s.go_view(v)));
        }
        return true;
    }
    false
}

// --------------------------------------------------------------------- disk

struct Kind {
    name: &'static str,
    color: Color32,
    bytes: f64,
    note: String,
    view: View,
}

fn kinds(u: &Usage) -> Vec<Kind> {
    let unused = |n: usize, b: f64| if n == 0 { String::new() } else { format!("{n} unused · {}", fmt_bytes(b)) };
    let img_n = u.images_unused.n + u.dangling.n + u.distros_unused.n;
    let img_b = u.images_unused.bytes + u.dangling.bytes + u.distros_unused.bytes;
    vec![
        Kind { name: "Images", color: C_IMAGES, bytes: u.images.bytes, note: unused(img_n, img_b), view: View::Images },
        Kind {
            name: "Containers",
            color: C_CONTAINERS,
            bytes: u.containers.bytes + u.pods.bytes,
            note: if u.stopped.n > 0 { format!("{} stopped · {}", u.stopped.n, fmt_bytes(u.stopped.bytes)) } else { String::new() },
            view: View::Containers,
        },
        Kind {
            name: "Machines",
            color: C_MACHINES,
            bytes: u.machines.bytes,
            note: if u.machines.n > 0 { format!("{} {}", u.machines.n, if u.machines.n == 1 { "machine" } else { "machines" }) } else { String::new() },
            view: View::Machines,
        },
        Kind { name: "Volumes", color: C_VOLUMES, bytes: u.volumes.bytes, note: unused(u.volumes_unused.n, u.volumes_unused.bytes), view: View::Volumes },
        Kind {
            name: "Build cache",
            color: C_CACHE,
            bytes: u.cache.bytes,
            note: if u.cache_unused.bytes > 0.0 { format!("{} reclaimable", fmt_bytes(u.cache_unused.bytes)) } else { String::new() },
            view: View::Builds,
        },
    ]
}

fn disk_card(ui: &mut Ui, cx: &mut Cx, r: Rect, u: Option<&Usage>) {
    let (mut cui, r) = card(ui, cx, r, 0.0);
    let p = cui.painter().clone();
    title(&cui, r, "Disk");
    let x = r.left() + 22.0;
    let right = r.right() - 22.0;
    let mut y = r.top() + 20.0 + 18.0 + 8.0;
    let Some(u) = u else {
        // Numbers are on the way.
        w::skeleton(&p, Rect::from_min_size(pos2(x, y + 6.0), vec2(180.0, 44.0)), 8.0, cx.now);
        w::skeleton(&p, Rect::from_min_size(pos2(x, y + 104.0), vec2(right - x, 8.0)), 4.0, cx.now);
        for i in 0..5 {
            w::skeleton(&p, Rect::from_min_size(pos2(x, y + 136.0 + i as f32 * 30.0), vec2(160.0 + 30.0 * (i % 3) as f32, 12.0)), 6.0, cx.now);
        }
        return;
    };
    let total = u.total();
    let shown = cx.tw.f("dash-total", total as f32, 0.9, Ease::OUT) as f64;
    let hero = w::g_ls(&cui, &fmt_bytes(shown), semibold(48.0), TEXT, -1.2);
    w::put(&p, &hero, pos2(x, y + 28.0), Align2::LEFT_CENTER);
    y += 56.0 + 4.0;
    let sub = if cx.s.is_live() {
        let image = crate::engine::cleanup::disk_image_bytes();
        let limit = crate::engine::config::load().disk_gb;
        format!("Docker's share of a disk image using {} of up to {limit} GB on this Mac", fmt_bytes(image))
    } else {
        "Docker's share of a disk image using 23.4 of up to 64 GB on this Mac".into()
    };
    w::text(&p, &cui, x, y + 9.0, &sub, sans(13.0), MUTED);
    y += 18.0 + 18.0;

    // One whole, split by kind: a thin segmented bar; the list below names
    // each part and its size, so the bar carries no text.
    let ks = kinds(u);
    let bar = Rect::from_min_size(pos2(x, y), vec2(right - x, 8.0));
    let grow = cx.tw.f("dash-bar", 1.0, 0.9, Ease::OUT);
    let parts: Vec<&Kind> = ks.iter().filter(|k| k.bytes > 0.0).collect();
    let sum: f64 = parts.iter().map(|k| k.bytes).sum();
    if sum <= 0.0 {
        w::fill(&p, bar, 4.0, TRACK);
    } else {
        let gaps = 2.0 * (parts.len().saturating_sub(1)) as f32;
        let avail = (bar.width() - gaps) * grow;
        let mut bx = bar.left();
        for (i, k) in parts.iter().enumerate() {
            let wseg = (avail * (k.bytes / sum) as f32).max(2.0);
            let seg = Rect::from_min_size(pos2(bx, bar.top()), vec2(wseg, bar.height()));
            let first = i == 0;
            let last = i + 1 == parts.len();
            let rad = egui::CornerRadius { nw: if first { 4 } else { 1 }, sw: if first { 4 } else { 1 }, ne: if last { 4 } else { 1 }, se: if last { 4 } else { 1 } };
            p.rect_filled(seg, rad, k.color);
            let hit = cui.interact(seg.expand2(vec2(0.0, 8.0)), Id::new(("dash-seg", i)), Sense::hover());
            hit.on_hover_text_at_pointer(format!("{} · {} · {:.0}%", k.name, fmt_bytes(k.bytes), k.bytes / sum * 100.0));
            bx += wseg + 2.0;
        }
    }
    y += 8.0 + 16.0;

    // The parts: swatch, name, size, and what's unused.
    let size_x = x + 150.0;
    for (i, k) in ks.iter().enumerate() {
        let row = Rect::from_min_size(pos2(x - 8.0, y), vec2(right - x + 16.0, 30.0));
        hover_row(&mut cui, cx, row, Id::new(("dash-kind", i)), Some(k.view));
        let cy = row.center().y;
        w::fill(&p, Rect::from_center_size(pos2(x + 5.0, cy), vec2(10.0, 10.0)), 3.0, k.color);
        w::text(&p, &cui, x + 20.0, cy, k.name, sans(13.5), TEXT);
        w::text(&p, &cui, size_x, cy, &fmt_bytes(k.bytes), mono(12.5), TEXT);
        if !k.note.is_empty() {
            w::text_right(&p, &cui, right, cy, &k.note, sans(12.5), MUTED);
        }
        y += 30.0;
    }

    // What Clean up could free.
    let foot = r.bottom() - 54.0;
    p.line_segment([pos2(r.left(), foot + 0.5), pos2(r.right(), foot + 0.5)], Stroke::new(1.0_f32, HAIRLINE));
    let cy = foot + 27.0;
    let free = u.reclaimable();
    let label = w::text(&p, &cui, x, cy, "Reclaimable", sans(13.5), TEXT2);
    w::text(&p, &cui, label.right() + 10.0, cy, &fmt_bytes(free), semibold(15.0), if free > 0.0 { TEXT } else { MUTED });
    let btn = if free > 0.0 { Btn::cta("Clean up") } else { Btn::ghost("Clean up") };
    let (resp, _) = btn.h(32.0).pad(14.0).radius(8.0).at_right(&mut cui, right, cy, Id::new("dash-clean"));
    if resp.clicked() {
        cx.fx.push(Box::new(|s| s.open_cleanup()));
    }
}

// ------------------------------------------------------------------- engine

fn engine_card(ui: &mut Ui, cx: &mut Cx, r: Rect) {
    let (mut cui, r) = card(ui, cx, r, 0.08);
    let p = cui.painter().clone();
    title(&cui, r, "Engine");
    let s = &*cx.s;
    let x = r.left() + 22.0;
    let right = r.right() - 22.0;
    let mut y = r.top() + 20.0 + 18.0 + 14.0;

    let (st, word) = match s.engine {
        Engine::Running | Engine::Ready => (St::Running, "Running"),
        Engine::Booting => (St::Starting, "Starting"),
        Engine::Off => (St::Stopped, "Stopped"),
        Engine::Failed => (St::Error, "Couldn't start"),
    };
    cx.ambient |= w::status_dot(&p, cx.tw, Id::new("dash-eng"), pos2(x + 5.0, y + 10.0), st, false, -10.0);
    w::text(&p, &cui, x + 20.0, y + 10.0, word, medium(17.0), TEXT);
    y += 20.0 + 6.0;
    let i = &s.info;
    let sub = if s.is_live() && !i.version.is_empty() {
        format!("Docker {} on {}", i.version, i.runtime.replace(" GNU/Linux", ""))
    } else if s.is_live() {
        "Docker Engine in Patina's VM".into()
    } else {
        "Docker 27.4 on Debian 13".into()
    };
    w::text(&p, &cui, x + 20.0, y + 9.0, &sub, sans(13.0), MUTED);
    y += 18.0 + 22.0;

    // Memory: a meter, accent fill on a track of the same family.
    let mem_gb = if i.mem_gb > 0.0 { i.mem_gb } else { s.saved.1 as f64 };
    let used = s.used_gb();
    w::text(&p, &cui, x, y + 8.0, "Memory", sans(13.0), TEXT2);
    w::text_right(&p, &cui, right, y + 8.0, &format!("{:.1} of {:.0} GB", used, mem_gb.max(1.0)), mono(12.5), TEXT);
    y += 16.0 + 8.0;
    let frac = cx.tw.f("dash-mem", (used / mem_gb.max(0.1)).clamp(0.0, 1.0) as f32, 0.9, Ease::OUT);
    w::bar(&p, Rect::from_min_size(pos2(x, y), vec2(right - x, 6.0)), frac, VERDIGRIS, VERDIGRIS.gamma_multiply(0.18));
    y += 6.0 + 18.0;

    // The disk image: what macOS has allocated of its limit.
    let (image, limit) = if s.is_live() {
        (crate::engine::cleanup::disk_image_bytes(), crate::engine::config::load().disk_gb as f64 * 1e9)
    } else {
        (23.4e9, 64e9)
    };
    w::text(&p, &cui, x, y + 8.0, "Disk image", sans(13.0), TEXT2);
    w::text_right(&p, &cui, right, y + 8.0, &format!("{} of {:.0} GB", fmt_bytes(image), limit / 1e9), mono(12.5), TEXT);
    y += 16.0 + 8.0;
    let dfrac = cx.tw.f("dash-img", (image / limit.max(1.0)).clamp(0.0, 1.0) as f32, 0.9, Ease::OUT);
    w::bar(&p, Rect::from_min_size(pos2(x, y), vec2(right - x, 6.0)), dfrac, VERDIGRIS, VERDIGRIS.gamma_multiply(0.18));
    y += 6.0 + 18.0;

    let on = |b: bool| if b { "On" } else { "Off" };
    let cpus = if i.cpus > 0 { i.cpus } else { s.saved.0 };
    let arch = if i.arch.is_empty() { "arm64".to_string() } else { i.arch.clone() };
    let home = crate::engine::paths::home().display().to_string();
    let sock = crate::engine::paths::docker_sock().display().to_string().replacen(&home, "~", 1);
    let rows: Vec<(&str, String, Option<View>)> = vec![
        ("CPUs", format!("{cpus} cores · {arch}"), Some(View::Settings)),
        ("Kubernetes", if s.kube == Kube::On { "On".into() } else { on(false).into() }, Some(View::Kubernetes)),
        ("Rosetta for Intel images", on(s.cfg_on("rosetta") || !s.is_live()).into(), Some(View::Settings)),
        ("Docker endpoint", sock, None),
        ("Patina", env!("CARGO_PKG_VERSION").to_string(), Some(View::Settings)),
    ];
    for (k, (label, value, view)) in rows.into_iter().enumerate() {
        let row = Rect::from_min_size(pos2(x - 8.0, y), vec2(right - x + 16.0, 34.0));
        if k > 0 {
            p.line_segment([pos2(x, y + 0.5), pos2(right, y + 0.5)], Stroke::new(1.0_f32, HAIRLINE));
        }
        if view.is_some() {
            hover_row(&mut cui, cx, row, Id::new(("dash-eng-row", k)), view);
        }
        let cy = row.center().y;
        w::text(&p, &cui, x, cy, label, sans(13.0), TEXT2);
        let g = w::g_elide(&cui, &value, mono(12.5), TEXT, right - x - 150.0);
        w::put(&p, &g, pos2(right, cy), Align2::RIGHT_CENTER);
        y += 34.0;
    }
}

// -------------------------------------------------------------------- tiles

fn tiles(ui: &mut Ui, cx: &mut Cx, x0: f32, y0: f32, wd: f32, u: Option<&Usage>) -> f32 {
    let s = &*cx.s;
    let boxes: Vec<_> = s.alive().collect();
    let running = boxes.iter().filter(|c| c.st == St::Running).count();
    let imgs = s.imgs.iter().filter(|a| !a.gone).count();
    let vols = s.vols.iter().filter(|a| !a.gone).count();
    let nets = s.nets.iter().filter(|n| !n.gone && !n.sys).count();
    let ms: Vec<_> = s.ms.iter().filter(|m| !m.gone).collect();
    let ms_up = ms.iter().filter(|m| m.st == St::Running).count();
    let pods = s.pods.iter().filter(|p| p.shown || !s.is_live()).count();
    let img_unused = u.map_or(0, |u| u.images_unused.n + u.dangling.n + u.distros_unused.n);
    let img_bytes = u.map_or(0.0, |u| u.images.bytes);
    let vol_unused = u.map_or(0, |u| u.volumes_unused.n);
    let vol_bytes = u.map_or(0.0, |u| u.volumes.bytes);
    let net_unused = u.map_or(0, |u| u.networks_unused);
    let stat: Vec<(&str, &'static str, String, String, View)> = vec![
        ("Containers", icons::CONTAINERS, boxes.len().to_string(), format!("{running} running · {} stopped", boxes.len() - running), View::Containers),
        ("Images", icons::IMAGES, imgs.to_string(), if img_unused > 0 { format!("{img_unused} unused · {}", fmt_bytes(img_bytes)) } else { fmt_bytes(img_bytes) }, View::Images),
        ("Volumes", icons::VOLUMES, vols.to_string(), if vol_unused > 0 { format!("{vol_unused} unused · {}", fmt_bytes(vol_bytes)) } else { fmt_bytes(vol_bytes) }, View::Volumes),
        ("Networks", icons::NETWORKS, nets.to_string(), if net_unused > 0 { format!("{net_unused} unused") } else { "all in use".into() }, View::Networks),
        ("Machines", icons::MACHINES, ms.len().to_string(), format!("{ms_up} running"), View::Machines),
        (
            "Kubernetes",
            icons::KUBERNETES,
            if s.kube == Kube::On { pods.to_string() } else { "Off".into() },
            if s.kube == Kube::On { format!("{} running", if pods == 1 { "pod" } else { "pods" }) } else { "not turned on".into() },
            View::Kubernetes,
        ),
    ];
    let min_w = 150.0;
    let cols = (((wd + GAP) / (min_w + GAP)).floor() as usize).clamp(2, stat.len());
    let tw = (wd - GAP * (cols - 1) as f32) / cols as f32;
    let th = 112.0;
    let rows = stat.len().div_ceil(cols);
    for (i, (label, icon, value, sub, view)) in stat.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x0 + (i % cols) as f32 * (tw + GAP), y0 + (i / cols) as f32 * (th + GAP)), vec2(tw, th));
        let (cui, r) = card(ui, cx, r, 0.12 + i as f32 * 0.03);
        let p = cui.painter().clone();
        let resp = cui.interact(r, Id::new(("dash-tile", i)), Sense::click());
        let hv = cx.tw.f(("dash-tile-hv", i), if resp.hovered() { 1.0 } else { 0.0 }, 0.2, Ease::EASE);
        if hv > 0.01 {
            w::edge(&p, r, 14.0, mix(CARD_EDGE, BTN_EDGE, hv));
        }
        if resp.hovered() {
            cui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.clicked() {
            cx.fx.push(Box::new(move |s| s.go_view(view)));
        }
        let x = r.left() + 18.0;
        icons::paint(&cui, Rect::from_center_size(pos2(x + 8.0, r.top() + 18.0 + 8.0), vec2(16.0, 16.0)), icon, Ink::Stroke(1.4), mix(MUTED, TEXT2, hv));
        w::text(&p, &cui, x + 24.0, r.top() + 18.0 + 8.0, label, sans(13.0), TEXT2);
        let vg = w::g_ls(&cui, &value, semibold(28.0), TEXT, -0.6);
        w::put(&p, &vg, pos2(x, r.top() + 18.0 + 16.0 + 12.0 + 17.0), Align2::LEFT_CENTER);
        let sg = w::g_elide(&cui, &sub, sans(12.5), MUTED, r.width() - 36.0);
        w::put(&p, &sg, pos2(x, r.bottom() - 18.0 - 8.0), Align2::LEFT_CENTER);
    }
    y0 + rows as f32 * th + (rows - 1) as f32 * GAP
}

// ----------------------------------------------------------------- activity

fn spark_card(ui: &mut Ui, cx: &mut Cx, r: Rect, cpu: bool, delay: f32) {
    let (cui, r) = card(ui, cx, r, delay);
    let p = cui.painter().clone();
    let s = &*cx.s;
    title(&cui, r, if cpu { "CPU" } else { "Memory" });
    let x = r.left() + 22.0;
    let right = r.right() - 22.0;
    let hist = if cpu { &s.cpu_hist } else { &s.mem_hist };
    let now = hist.last().copied().unwrap_or(0.0);
    let running = s.alive().filter(|c| c.st == St::Running).count();
    let (value, sub) = if cpu {
        let cores = if s.info.cpus > 0 { s.info.cpus } else { s.saved.0 }.max(1);
        (format!("{now:.0}%"), format!("of one core · {cores} cores · {running} running"))
    } else {
        let gb = if s.info.mem_gb > 0.0 { s.info.mem_gb } else { s.saved.1 as f64 };
        (fmt_bytes(now as f64 * 1_048_576.0), format!("used by containers · {gb:.0} GB for the engine"))
    };
    let vg = w::g_ls(&cui, &value, semibold(28.0), TEXT, -0.6);
    w::put(&p, &vg, pos2(x, r.top() + 20.0 + 18.0 + 8.0 + 17.0), Align2::LEFT_CENTER);
    let sg = w::g_elide(&cui, &sub, sans(12.5), MUTED, right - x);
    w::put(&p, &sg, pos2(x, r.top() + 20.0 + 18.0 + 8.0 + 34.0 + 12.0), Align2::LEFT_CENTER);

    // The last minute, newest at the right: one series, a 2px line over a
    // faint area, with a crosshair and the value under the pointer.
    let area = Rect::from_min_max(pos2(x, r.top() + 112.0), pos2(right, r.bottom() - 18.0));
    p.line_segment([pos2(area.left(), area.bottom() + 0.5), pos2(area.right(), area.bottom() + 0.5)], Stroke::new(1.0_f32, HAIRLINE));
    if hist.len() < 2 {
        w::text(&p, &cui, area.left(), area.center().y, "Collecting the last minute", sans(12.5), DIM);
        return;
    }
    let max = hist.iter().copied().fold(if cpu { 10.0 } else { 64.0 }, f32::max) * 1.15;
    let step = area.width() / 59.0;
    let pts: Vec<egui::Pos2> = hist
        .iter()
        .enumerate()
        .map(|(i, v)| pos2(area.right() - (hist.len() - 1 - i) as f32 * step, area.bottom() - (v / max) * area.height()))
        .collect();
    let mut fill = pts.clone();
    fill.push(pos2(pts.last().unwrap().x, area.bottom()));
    fill.push(pos2(pts[0].x, area.bottom()));
    p.add(Shape::convex_polygon(fill, VERDIGRIS.gamma_multiply(0.10), Stroke::NONE));
    p.add(Shape::line(pts.clone(), Stroke::new(2.0_f32, VERDIGRIS)));
    let resp = cui.interact(area.expand(6.0), Id::new(("dash-spark", cpu)), Sense::hover());
    if let Some(pos) = resp.hover_pos() {
        let i = pts.iter().enumerate().min_by(|a, b| (a.1.x - pos.x).abs().total_cmp(&(b.1.x - pos.x).abs())).map(|(i, _)| i).unwrap_or(0);
        let pt = pts[i];
        p.line_segment([pos2(pt.x, area.top()), pos2(pt.x, area.bottom())], Stroke::new(1.0_f32, TEXT2.gamma_multiply(0.4)));
        p.circle_filled(pt, 4.0, CARD);
        p.circle_filled(pt, 3.0, VERDIGRIS);
        let ago = hist.len() - 1 - i;
        let v = hist[i];
        let when = if ago == 0 { "now".to_string() } else { format!("{ago} s ago") };
        let what = if cpu { format!("{v:.1}%") } else { fmt_bytes(v as f64 * 1_048_576.0) };
        resp.on_hover_text_at_pointer(format!("{what} · {when}"));
    } else {
        p.circle_filled(*pts.last().unwrap(), 3.0, VERDIGRIS);
    }
}

fn busiest(ui: &mut Ui, cx: &mut Cx, r: Rect) {
    let (mut cui, r) = card(ui, cx, r, 0.38);
    let p = cui.painter().clone();
    title(&cui, r, "Busiest containers");
    let x = r.left() + 22.0;
    let right = r.right() - 22.0;
    let mut list: Vec<(String, f64, f64)> = cx.s.alive().filter(|c| c.st == St::Running).map(|c| (c.id.clone(), c.cpu, c.mem)).collect();
    list.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.total_cmp(&a.2)));
    list.truncate(5);
    let mut y = r.top() + 20.0 + 18.0 + 10.0;
    if list.is_empty() {
        w::text(&p, &cui, x, y + 40.0, "No containers are running.", sans(13.0), MUTED);
        return;
    }
    let mem_x = right;
    let cpu_x = right - 86.0;
    w::text_right(&p, &cui, cpu_x, y + 7.0, "CPU", sans(11.5), DIM);
    w::text_right(&p, &cui, mem_x, y + 7.0, "Memory", sans(11.5), DIM);
    y += 16.0;
    let top = list[0].1.max(1.0);
    for (i, (id, cpu, mem)) in list.into_iter().enumerate() {
        let row = Rect::from_min_size(pos2(x - 8.0, y), vec2(right - x + 16.0, 30.0));
        if hover_row(&mut cui, cx, row, Id::new(("dash-busy", i)), None) {
            let id = id.clone();
            cx.fx.push(Box::new(move |s| {
                s.go_view(View::Containers);
                s.pick(&id);
            }));
        }
        let cy = row.center().y;
        let name_w = cpu_x - 96.0 - x;
        let g = w::g_elide(&cui, &id, sans(13.0), TEXT, name_w.max(40.0));
        w::put(&p, &g, pos2(x, cy), Align2::LEFT_CENTER);
        // A small meter of its CPU against the busiest one.
        let m = Rect::from_min_size(pos2(cpu_x - 96.0 + 8.0, cy - 2.0), vec2(40.0, 4.0));
        w::bar(&p, m, (cpu / top) as f32, VERDIGRIS, VERDIGRIS.gamma_multiply(0.18));
        w::text_right(&p, &cui, cpu_x, cy, &format!("{cpu:.1}%"), mono(12.5), TEXT);
        w::text_right(&p, &cui, mem_x, cy, &fmt_bytes(mem * 1_048_576.0), mono(12.5), TEXT2);
        y += 30.0;
    }
}

// -------------------------------------------------------------------- lists

/// Ranked bars, largest first, in one hue: the parts compared with each other.
#[allow(clippy::too_many_arguments)]
fn ranked(ui: &mut Ui, cx: &mut Cx, r: Rect, name: &str, items: &[Item], color: Color32, view: View, empty: &str, delay: f32) {
    let (mut cui, r) = card(ui, cx, r, delay);
    let p = cui.painter().clone();
    title(&cui, r, name);
    let x = r.left() + 22.0;
    let right = r.right() - 22.0;
    let mut y = r.top() + 20.0 + 18.0 + 12.0;
    if items.is_empty() {
        w::text(&p, &cui, x, y + 30.0, empty, sans(13.0), MUTED);
        return;
    }
    let top = items[0].bytes.max(1.0);
    let grow = cx.tw.f(("dash-rank", name), 1.0, 0.9, Ease::OUT);
    for (i, it) in items.iter().take(6).enumerate() {
        let row = Rect::from_min_size(pos2(x - 8.0, y), vec2(right - x + 16.0, 36.0));
        hover_row(&mut cui, cx, row, Id::new(("dash-rank-row", name, i)), Some(view));
        let size = fmt_bytes(it.bytes);
        let size_w = w::text_w(&cui, &size, mono(12.0));
        let tag = if it.used { "" } else { "unused" };
        let tag_w = if tag.is_empty() { 0.0 } else { w::text_w(&cui, tag, sans(11.5)) + 10.0 };
        let g = w::g_elide(&cui, &it.name, sans(13.0), TEXT, right - x - size_w - tag_w - 12.0);
        w::put(&p, &g, pos2(x, y + 11.0), Align2::LEFT_CENTER);
        if !tag.is_empty() {
            w::text(&p, &cui, x + g.size().x + 8.0, y + 11.0, tag, sans(11.5), MUTED);
        }
        w::text_right(&p, &cui, right, y + 11.0, &size, mono(12.0), TEXT2);
        let bar = Rect::from_min_size(pos2(x, y + 24.0), vec2((right - x) * (it.bytes / top) as f32 * grow, 4.0));
        p.rect_filled(bar, 2.0, if it.used { color } else { color.gamma_multiply(0.45) });
        y += 36.0;
    }
}

/// Open ports and running machines, one click from the browser.
fn links(ui: &mut Ui, cx: &mut Cx, r: Rect) {
    let (mut cui, r) = card(ui, cx, r, 0.5);
    let p = cui.painter().clone();
    title(&cui, r, "Open in the browser");
    let x = r.left() + 22.0;
    let right = r.right() - 22.0;
    let mut y = r.top() + 20.0 + 18.0 + 12.0;
    let s = &*cx.s;
    let named = !s.info.stale;
    let mut list: Vec<(String, String, String)> = s
        .alive()
        .filter(|c| c.st == St::Running && !c.port.is_empty())
        .map(|c| (format!("localhost:{}", c.port), c.id.clone(), format!("http://localhost:{}", c.port)))
        .collect();
    for m in s.ms.iter().filter(|m| !m.gone && m.st == St::Running && !m.ip.is_empty()) {
        let host = if named || !s.is_live() { crate::engine::machines::domain(&m.id) } else { m.ip.clone() };
        list.push((host.clone(), format!("{} machine", m.distro), format!("http://{host}")));
    }
    if list.is_empty() {
        let g = w::g_wrap(&cui, "Publish a port or start a machine, and its link shows up here.", sans(13.0), MUTED, right - x, 18.0);
        p.galley(pos2(x, y + 20.0), g, MUTED);
        return;
    }
    for (i, (host, what, url)) in list.into_iter().take(7).enumerate() {
        let row = Rect::from_min_size(pos2(x - 8.0, y), vec2(right - x + 16.0, 32.0));
        if hover_row(&mut cui, cx, row, Id::new(("dash-link", i)), None) {
            cx.fx.push(Box::new(move |s: &mut State| s.open_url(url)));
        }
        let cy = row.center().y;
        let hr = w::text(&p, &cui, x, cy, &host, mono(12.5), VERDIGRIS);
        let g = w::g_elide(&cui, &what, sans(12.5), MUTED, right - hr.right() - 12.0);
        w::put(&p, &g, pos2(right, cy), Align2::RIGHT_CENTER);
        y += 32.0;
    }
}
