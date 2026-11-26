//! Settings panes: switches, sliders, read-only values, and the
//! "apply and restart" bar for resource changes.

use egui::{pos2, vec2, Align2, Id, Rect, Stroke, Ui, UiBuilder};

use super::{swap, Cx};
use crate::sim::{Pane, Settings, State, Upd};
use crate::theme::*;
use crate::widgets::{self as w, Btn};

enum Ctl {
    Toggle(fn(&mut Settings) -> &mut bool),
    /// An engine setting: config key, and whether it needs an engine restart.
    Cfg(&'static str, bool),
    Slider(fn(&mut Settings) -> &mut u32, u32, u32, &'static str),
    Text(String),
    Code(String),
    Update,
    /// The terminal app shells open in, from those on this Mac.
    Terminal,
}

struct RowDef {
    label: &'static str,
    hint: String,
    ctl: Ctl,
}

fn row(label: &'static str, hint: impl Into<String>, ctl: Ctl) -> RowDef {
    RowDef { label, hint: hint.into(), ctl }
}

fn rows(s: &State) -> Vec<RowDef> {
    if s.is_live() {
        return live_rows(s);
    }
    match s.pane {
        Pane::General => vec![
            row("Open at login", "Start Patina when you sign in to this Mac.", Ctl::Toggle(|s| &mut s.login)),
            row("Show in the menu bar", "Keep the status icon and quick controls in the menu bar.", Ctl::Toggle(|s| &mut s.menubar)),
            row("Start the engine on launch", "When off, the engine starts the first time you run a docker command.", Ctl::Toggle(|s| &mut s.autostart)),
            row("Install updates automatically", "Updates are applied the next time the engine restarts.", Ctl::Toggle(|s| &mut s.autoupdate)),
        ],
        Pane::Resources => vec![
            row("CPU limit", "Cores the engine may use, out of 10 on this Mac.", Ctl::Slider(|s| &mut s.cpus, 1, 10, "cores")),
            row("Memory limit", "Memory the engine does not need is returned to macOS.", Ctl::Slider(|s| &mut s.mem, 2, 32, "GB")),
            row("Disk image", "Grows as needed, up to the limit.", Ctl::Text("23.4 of 64 GB".into())),
        ],
        Pane::Network => vec![
            row("Publish ports on localhost", "A container port you publish answers on localhost on this Mac.", Ctl::Toggle(|s| &mut s.ports)),
            row("Follow macOS DNS and VPN", "Containers resolve names and reach private networks the way this Mac does.", Ctl::Toggle(|s| &mut s.dns)),
            row("Default subnet", "Used by the bridge network.", Ctl::Text("172.17.0.0/16".into())),
        ],
        Pane::Engine => vec![
            row("Run Intel images with Rosetta", "Used for linux/amd64 images on Apple silicon.", Ctl::Toggle(|s| &mut s.rosetta)),
            row(
                "daemon.json",
                "Docker Engine configuration. Applied after a restart.",
                Ctl::Code("{\n  \"features\": { \"buildkit\": true },\n  \"experimental\": false\n}".into()),
            ),
        ],
        Pane::About => vec![
            row("Version", "Patina for Apple silicon.", Ctl::Text(env!("CARGO_PKG_VERSION").into())),
            row(
                "Updates",
                match s.upd {
                    Upd::Current => "Patina is up to date.",
                    Upd::Checking => "Checking for updates",
                    Upd::Idle => "Last checked today.",
                },
                Ctl::Update,
            ),
            row("Engine", "Runs inside one small Linux VM.", Ctl::Text("Docker Engine".into())),
        ],
    }
}

/// Settings for Patina's own engine, stored in `~/.patina/config.json`.
fn live_rows(s: &State) -> Vec<RowDef> {
    if crate::engine::NATIVE {
        return native_rows(s);
    }
    use crate::engine::config::{host_cpus, host_mem_gb};
    let i = &s.info;
    let or = |v: &str, d: &str| if v.is_empty() { d.to_string() } else { v.to_string() };
    let disk = if i.disk_gb > 0.0 { format!("{:.1} GB", i.disk_gb) } else { "—".into() };
    let home = crate::engine::paths::home().display().to_string();
    let short = |p: std::path::PathBuf| p.display().to_string().replacen(&home, "~", 1);
    match s.pane {
        Pane::General => vec![
            row("Open at login", "Start Patina when you sign in to this Mac.", Ctl::Cfg("login", false)),
            row(
                "Show in the menu bar",
                "Quick controls in the menu bar. Closing the window keeps Patina running there.",
                Ctl::Cfg("menubar", false),
            ),
            row("Start the engine on launch", "When off, start it from the engine card.", Ctl::Cfg("autostart", false)),
            row(
                "Make Patina the docker default",
                "The docker command uses Patina's engine (the patina context) each time the engine starts.",
                Ctl::Cfg("set_context", false),
            ),
            row(
                "Add Patina's tools to PATH",
                "Puts ~/.patina/bin first in new terminals, through one line in your shell profile.",
                Ctl::Cfg("shell_path", false),
            ),
            row("Terminal", "Where Open in Terminal opens shells, for containers and machines.", Ctl::Terminal),
        ],
        Pane::Resources => {
            let cores = host_cpus().max(s.saved.0).max(1);
            let mem = host_mem_gb().max(s.saved.1).max(2);
            vec![
                row("CPU limit", format!("Cores the engine may use, out of {cores} on this Mac."), Ctl::Slider(|s| &mut s.cpus, 1, cores, "cores")),
                row("Memory limit", "Memory the engine does not need is returned to macOS.", Ctl::Slider(|s| &mut s.mem, 2, mem, "GB")),
                row("Disk usage", "Images, containers and volumes, on a disk image that grows as needed.", Ctl::Text(disk)),
            ]
        }
        Pane::Network => vec![
            row("Published ports", "A container port you publish answers on localhost on this Mac.", Ctl::Text("localhost".into())),
            row("Docker endpoint", "Point any Docker client here, or use the patina context.", Ctl::Text(short(crate::engine::paths::docker_sock()))),
        ],
        Pane::Engine => vec![
            row("Run Intel images with Rosetta", "Used for linux/amd64 images on Apple silicon.", Ctl::Cfg("rosetta", true)),
            row("Docker Engine", "Version reported by the engine.", Ctl::Text(or(&i.version, "—"))),
            row("Linux", "The engine's guest system, in its own VM.", Ctl::Text(or(&i.runtime, "—"))),
        ],
        Pane::About => vec![
            row("Version", "Patina for Apple silicon.", Ctl::Text(env!("CARGO_PKG_VERSION").into())),
            row("Command line", "Add this folder to your PATH to use docker and docker-compose.", Ctl::Text(short(crate::engine::paths::bin()))),
            row("Data", "The disk image, logs and settings.", Ctl::Text(short(crate::engine::paths::root()))),
        ],
    }
}

/// Settings with this computer's own Docker Engine (Linux). Patina doesn't
/// own that engine, so there are no limits, Rosetta or docker context to set.
fn native_rows(s: &State) -> Vec<RowDef> {
    let i = &s.info;
    let or = |v: &str, d: &str| if v.is_empty() { d.to_string() } else { v.to_string() };
    let disk = if i.disk_gb > 0.0 { format!("{:.1} GB", i.disk_gb) } else { "—".into() };
    let home = crate::engine::paths::home().display().to_string();
    let short = |p: std::path::PathBuf| p.display().to_string().replacen(&home, "~", 1);
    match s.pane {
        Pane::General => vec![
            row("Open at login", "Start Patina when you sign in.", Ctl::Cfg("login", false)),
            if crate::terminal::installed().is_empty() {
                row("Terminal", "Open in Terminal needs a terminal app, such as GNOME Terminal or Konsole.", Ctl::Text("None found".into()))
            } else {
                row("Terminal", "Where Open in Terminal opens shells, for containers.", Ctl::Terminal)
            },
        ],
        Pane::Resources => vec![
            row("CPUs", "Docker Engine runs on this computer and may use all of it.", Ctl::Text(format!("{} cores", i.cpus))),
            row("Disk usage", "Images, containers, volumes and build cache.", Ctl::Text(disk)),
        ],
        Pane::Network => vec![
            row("Published ports", "A container port you publish answers on this computer.", Ctl::Text("localhost".into())),
            row("Docker endpoint", "The Docker Engine socket Patina uses. Set DOCKER_HOST to choose another.", Ctl::Text(short(crate::engine::paths::docker_sock()))),
        ],
        Pane::Engine => vec![
            row("Docker Engine", "Version reported by the engine.", Ctl::Text(or(&i.version, "—"))),
            row("Linux", "This computer's system, which the engine runs on.", Ctl::Text(or(&i.runtime, "—"))),
        ],
        Pane::About => vec![
            row("Version", "Patina for Linux.", Ctl::Text(env!("CARGO_PKG_VERSION").into())),
            row("Data", "Settings and logs.", Ctl::Text(short(crate::engine::paths::root()))),
        ],
    }
}

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let wd = ui.available_width().min(700.0);
    let origin = ui.cursor().min;
    let defs = rows(cx.s);
    let (a, dy) = swap(cx.tw, cx.s.tab_at, 0.35);

    // Measure rows first so the card can be painted behind them.
    let label_w = wd - 40.0 - 20.0 - 260.0;
    let mut heights = Vec::new();
    for d in &defs {
        let hint = w::g_wrap(ui, &d.hint, sans(12.5), MUTED, label_w.max(200.0), 17.5);
        let mut h = 32.0 + 19.0 + 2.0 + hint.size().y;
        if let Ctl::Code(code) = &d.ctl {
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
            Ctl::Cfg(key, restart) => {
                let on = cx.s.cfg_on(key);
                let r = Rect::from_min_size(pos2(right - 40.0, cy - 12.0), vec2(40.0, 24.0));
                if w::switch(&mut sui, cx.tw, r, id, on, if on { VERDIGRIS } else { SWITCH_OFF }).clicked() {
                    let (key, restart) = (*key, *restart);
                    cx.fx.push(Box::new(move |s| s.set_cfg(key, !on, restart)));
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
            Ctl::Terminal => {
                let apps = crate::terminal::installed();
                let labels: Vec<&str> = apps.iter().map(|a| a.1).collect();
                let cur = apps.iter().position(|a| a.0 == cx.s.terminal_app()).unwrap_or(0);
                let sw = (labels.len() as f32 * 78.0).min(right - x - 200.0).max(100.0);
                let r = Rect::from_min_size(pos2(right - sw, cy - 17.0), vec2(sw, 34.0));
                if let Some(i) = w::segmented(&mut sui, cx.tw, r, id, &labels, cur) {
                    if i != cur {
                        let app = apps[i].0;
                        cx.fx.push(Box::new(move |s| s.set_terminal(app)));
                    }
                }
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
