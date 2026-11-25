//! Settings panes: switches, sliders, read-only values, and the
//! "apply and restart" bar for resource changes.

use egui::{pos2, vec2, Align2, Id, Rect, Stroke, Ui, UiBuilder};

use super::{swap, Cx};
use crate::sim::{Pane, Settings, Upd};
use crate::theme::*;
use crate::widgets::{self as w, Btn};

enum Ctl {
    Toggle(fn(&mut Settings) -> &mut bool),
    Slider(fn(&mut Settings) -> &mut u32, u32, u32, &'static str),
    Text(&'static str),
    Code(&'static str),
    Update,
}

struct RowDef {
    label: &'static str,
    hint: String,
    ctl: Ctl,
}

fn row(label: &'static str, hint: &str, ctl: Ctl) -> RowDef {
    RowDef { label, hint: hint.into(), ctl }
}

fn rows(pane: Pane, upd: Upd) -> Vec<RowDef> {
    match pane {
        Pane::General => vec![
            row("Open at login", "Start Patina when you sign in to this Mac.", Ctl::Toggle(|s| &mut s.login)),
            row("Show in the menu bar", "Keep the status icon and quick controls in the menu bar.", Ctl::Toggle(|s| &mut s.menubar)),
            row("Start the engine on launch", "When off, the engine starts the first time you run a docker command.", Ctl::Toggle(|s| &mut s.autostart)),
            row("Install updates automatically", "Updates are applied the next time the engine restarts.", Ctl::Toggle(|s| &mut s.autoupdate)),
        ],
        Pane::Resources => vec![
            row("CPU limit", "Cores the engine may use, out of 10 on this Mac.", Ctl::Slider(|s| &mut s.cpus, 1, 10, "cores")),
            row("Memory limit", "Memory the engine does not need is returned to macOS.", Ctl::Slider(|s| &mut s.mem, 2, 32, "GB")),
            row("Disk image", "Grows as needed, up to the limit.", Ctl::Text("23.4 of 64 GB")),
        ],
        Pane::Network => vec![
            row("Publish ports on localhost", "A container port you publish answers on localhost on this Mac.", Ctl::Toggle(|s| &mut s.ports)),
            row("Follow macOS DNS and VPN", "Containers resolve names and reach private networks the way this Mac does.", Ctl::Toggle(|s| &mut s.dns)),
            row("Default subnet", "Used by the bridge network.", Ctl::Text("172.17.0.0/16")),
        ],
        Pane::Engine => vec![
            row("Run Intel images with Rosetta", "Used for linux/amd64 images on Apple silicon.", Ctl::Toggle(|s| &mut s.rosetta)),
            row(
                "daemon.json",
                "Docker Engine configuration. Applied after a restart.",
                Ctl::Code("{\n  \"features\": { \"buildkit\": true },\n  \"experimental\": false\n}"),
            ),
        ],
        Pane::About => vec![
            row("Version", "Patina for Apple silicon.", Ctl::Text(env!("CARGO_PKG_VERSION"))),
            row(
                "Updates",
                match upd {
                    Upd::Current => "Patina is up to date.",
                    Upd::Checking => "Checking for updates",
                    Upd::Idle => "Last checked today.",
                },
                Ctl::Update,
            ),
            row("Engine", "Runs inside one small Linux VM.", Ctl::Text("Docker Engine")),
        ],
    }
}

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let wd = ui.available_width().min(700.0);
    let origin = ui.cursor().min;
    let pane = cx.s.pane;
    let defs = rows(pane, cx.s.upd);
    let (a, dy) = swap(cx.tw, cx.s.tab_at, 0.35);

    // Measure rows first so the card can be painted behind them.
    let label_w = wd - 40.0 - 20.0 - 260.0;
    let mut heights = Vec::new();
    for d in &defs {
        let hint = w::g_wrap(ui, &d.hint, sans(12.5), MUTED, label_w.max(200.0), 17.5);
        let mut h = 32.0 + 19.0 + 2.0 + hint.size().y;
        if let Ctl::Code(code) = d.ctl {
            let cg = w::g_wrap(ui, code, mono(12.0), CODE, wd - 40.0 - 32.0, 20.4);
            h += 4.0 + 8.0 + cg.size().y + 28.0;
        }
        heights.push((h, hint));
    }
    let card_h: f32 = heights.iter().map(|h| h.0).sum();
    let dirty = cx.s.dirty_count();
    let total = card_h + if dirty > 0 { 18.0 + 46.0 } else { 0.0 };
    let area = Rect::from_min_size(origin + vec2(0.0, dy), vec2(wd, total));
    let mut sui = ui.new_child(UiBuilder::new().max_rect(area));
    sui.multiply_opacity(a);
    let p = sui.painter().clone();
    let card = Rect::from_min_size(area.min, vec2(wd, card_h));
    w::card(&p, card, 14.0, CARD, CARD_EDGE);

    let mut y = card.top();
    let x = card.left() + 20.0;
    let right = card.right() - 20.0;
    for (i, (d, (h, hint))) in defs.iter().zip(heights).enumerate() {
        if i > 0 {
            p.line_segment([pos2(card.left(), y + 0.5), pos2(card.right(), y + 0.5)], Stroke::new(1.0_f32, HAIRLINE));
        }
        let top = y + 16.0;
        let block_h = 19.0 + 2.0 + hint.size().y;
        w::text(&p, &sui, x, top + 9.5, d.label, medium(13.5), TEXT);
        p.galley(pos2(x, top + 21.0), hint.clone(), MUTED);
        let cy = top + block_h / 2.0;
        let id = Id::new(("set", d.label));
        match &d.ctl {
            Ctl::Toggle(get) => {
                let on = *get(&mut cx.s.set);
                let r = Rect::from_min_size(pos2(right - 40.0, cy - 12.0), vec2(40.0, 24.0));
                if w::switch(&mut sui, cx.tw, r, id, on, if on { VERDIGRIS } else { SWITCH_OFF }).clicked() {
                    let get = *get;
                    cx.fx.push(Box::new(move |s| {
                        let v = get(&mut s.set);
                        *v = !*v;
                    }));
                }
            }
            Ctl::Slider(get, min, max, unit) => {
                let mut v = *get(&mut cx.s.set);
                let val = format!("{v} {unit}");
                w::text_right(&p, &sui, right, cy, &val, mono(12.5), TEXT);
                let sr = Rect::from_min_max(pos2(right - 72.0 - 14.0 - 220.0, cy - 10.0), pos2(right - 72.0 - 14.0, cy + 10.0));
                if w::slider(&mut sui, sr, id, &mut v, *min, *max) {
                    *get(&mut cx.s.set) = v;
                }
            }
            Ctl::Text(v) => {
                w::text_right(&p, &sui, right, cy, v, mono(12.5), CODE);
            }
            Ctl::Code(code) => {
                let cg = w::g_wrap(&sui, code, mono(12.0), CODE, wd - 40.0 - 32.0, 20.4);
                let cr = Rect::from_min_size(pos2(x, top + block_h + 12.0), vec2(right - x, cg.size().y + 28.0));
                w::fill(&p, cr, 10.0, TERM);
                p.galley(pos2(cr.left() + 16.0, cr.top() + 14.0), cg, CODE);
            }
            Ctl::Update => {
                let checking = cx.s.upd == Upd::Checking;
                let current = cx.s.upd == Upd::Current;
                let b = Btn::ghost("Check now").h(30.0).pad(13.0).radius(8.0).lead(12.0);
                let (resp, br) = b.at_right(&mut sui, right, cy, id);
                let content = 12.0 + 8.0 + w::text_w(&sui, "Check now", medium(13.5));
                let mc = pos2(br.center().x - content / 2.0 + 6.0, cy);
                if checking {
                    w::spin_ring(&p, mc, 5.25, 1.5, rgba(212, 145, 92, 0.25), COPPER, cx.now, 0.8);
                    cx.ambient = true;
                } else if current {
                    w::status_dot(&p, cx.tw, Id::new("upd-mark"), mc, crate::sim::St::Running, false, cx.s.upd_at);
                } else {
                    p.circle_stroke(mc, 5.25, Stroke::new(1.5_f32, DIM));
                }
                if resp.clicked() {
                    cx.fx.push(Box::new(|s| s.check_upd()));
                }
            }
        }
        y += h;
    }

    if dirty > 0 {
        let br = Rect::from_min_size(pos2(area.left(), card.bottom() + 18.0), vec2(wd, 46.0));
        w::card(&p, br, 12.0, DIRTY_BG, WARN_EDGE);
        let cy = br.center().y;
        let msg = format!("Restart the engine to apply {}.", if dirty == 1 { "this change" } else { "these changes" });
        w::text(&p, &sui, br.left() + 18.0, cy, &msg, sans(13.5), TEXT);
        let (ar, arr) = Btn::cta("Apply and restart").h(30.0).pad(13.0).radius(8.0).at_right(&mut sui, br.right() - 8.0, cy, Id::new("set-apply"));
        if ar.clicked() {
            cx.fx.push(Box::new(|s| s.apply_set()));
        }
        let (dr, _) = Btn::ghost("Discard").h(30.0).pad(13.0).radius(8.0).at_right(&mut sui, arr.left() - 10.0, cy, Id::new("set-discard"));
        if dr.clicked() {
            cx.fx.push(Box::new(|s| s.discard_set()));
        }
    }
    let _ = Align2::LEFT_CENTER;
    ui.advance_cursor_after_rect(Rect::from_min_size(origin, vec2(wd, total)));
}
