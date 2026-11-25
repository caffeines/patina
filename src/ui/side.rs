//! Sidebar: brand, navigation with the sliding highlight, and the engine card.

use egui::{pos2, vec2, Align2, Context, CursorIcon, Frame, Id, Rect, Sense, Stroke, UiBuilder, ViewportCommand};

use super::{enter, Cx};
use crate::icons::{self, Ink};
use crate::sim::{Engine, View};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

pub fn show(ctx: &Context, cx: &mut Cx) {
    egui::SidePanel::left("side")
        .exact_width(232.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(SIDEBAR))
        .show(ctx, |ui| {
            let r = ui.max_rect();
            let p = ui.painter().clone();
            p.line_segment([pos2(r.right() - 0.5, r.top()), pos2(r.right() - 0.5, r.bottom())], Stroke::new(1.0_f32, HAIRLINE));

            // Drag the window by the empty top of the sidebar.
            let drag = ui.interact(Rect::from_min_size(r.min, vec2(r.width(), 46.0)), Id::new("side-drag"), Sense::click_and_drag());
            if drag.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
            }

            let x0 = r.left() + 14.0;
            let wd = r.width() - 28.0;
            let started = cx.started;
            let mut y = r.top() + if cfg!(target_os = "macos") { 58.0 } else { 22.0 };

            // Brand
            {
                let (a, dy) = enter(cx.tw, started, 0.0, 0.6);
                let mut bp = p.clone();
                bp.multiply_opacity(a);
                let cy = y + 11.0 + dy;
                w::logo(&bp, pos2(x0 + 6.0 + 11.0, cy), 10.0);
                let gal = w::g_ls(ui, "Patina", semibold(16.0), TEXT, -0.16);
                w::put(&bp, &gal, pos2(x0 + 6.0 + 22.0 + 10.0, cy), Align2::LEFT_CENTER);
            }
            y += 22.0 + 22.0;

            // Navigation
            let s = &*cx.s;
            let counts = [
                Some(s.alive().count()),
                Some(s.imgs.iter().filter(|a| !a.gone).count()),
                Some(s.vols.iter().filter(|a| !a.gone).count()),
                Some(s.nets.iter().filter(|a| !a.gone).count()),
                Some(s.ms.iter().filter(|a| !a.gone).count()),
                None,
                None,
            ];
            let cur = s.view;
            let nav_y = y;
            let hy = cx.tw.f("nav-hl", cur.index() as f32 * 40.0, 0.42, Ease::SPRING);
            let hl = Rect::from_min_size(pos2(x0, nav_y + hy), vec2(wd, 36.0));
            w::card(&p, hl, 9.0, NAV_ON, NAV_ON_EDGE);
            for (i, v) in View::ALL.into_iter().enumerate() {
                let rect = Rect::from_min_size(pos2(x0, nav_y + i as f32 * 40.0), vec2(wd, 36.0));
                let resp = ui.interact(rect, Id::new(("nav", i)), Sense::click());
                if resp.hovered() {
                    ctx.set_cursor_icon(CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    cx.fx.push(Box::new(move |s| s.go_view(v)));
                }
                let on = v == cur;
                let col = cx.tw.c(("nav-c", i), if on || resp.hovered() { TEXT } else { TEXT2 }, 0.25);
                let (a, dy) = enter(cx.tw, started, 0.05 + i as f32 * 0.04, 0.55);
                let mut ip = p.clone();
                ip.multiply_opacity(a);
                let cy = rect.center().y + dy;
                let mut iui = ui.new_child(UiBuilder::new().max_rect(rect));
                iui.multiply_opacity(a);
                icons::paint(&iui, Rect::from_center_size(pos2(x0 + 10.0 + 8.0, cy), vec2(16.0, 16.0)), icons::nav(v), Ink::Stroke(1.4), col);
                w::text(&ip, ui, x0 + 10.0 + 16.0 + 10.0, cy, v.label(), sans(13.5), col);
                if let Some(n) = counts[i] {
                    w::text_right(&ip, ui, rect.right() - 10.0, cy, &n.to_string(), mono(11.5), MUTED);
                }
            }

            // Engine card, pinned to the bottom.
            let card_h = 147.0;
            let card = Rect::from_min_size(pos2(x0, r.bottom() - 14.0 - card_h), vec2(wd, card_h));
            let (a, dy) = enter(cx.tw, started, 0.3, 0.6);
            let card = card.translate(vec2(0.0, dy));
            let mut cui = ui.new_child(UiBuilder::new().max_rect(card));
            cui.multiply_opacity(a);
            engine_card(&mut cui, cx, card);
        });
}

fn engine_card(ui: &mut egui::Ui, cx: &mut Cx, card: Rect) {
    let s = &*cx.s;
    let p = ui.painter().clone();
    w::card(&p, card, 12.0, ENGINE_CARD, CARD_EDGE);
    let eng = s.engine;
    let boot = eng == Engine::Booting;
    let live = s.live();
    let down = matches!(eng, Engine::Off | Engine::Failed);
    let x = card.left() + 14.0;
    let right = card.right() - 14.0;
    let mut y = card.top() + 14.0;

    // Ring + status
    let ring_off = if boot {
        [46.0, 30.0, 13.0][s.boot.min(2)]
    } else if eng == Engine::Off {
        56.6
    } else {
        0.0
    };
    let off = cx.tw.f("eng-ring", ring_off, 0.5, Ease::STD);
    let ring_c = cx.tw.c("eng-ring-c", if boot { COPPER } else if eng == Engine::Failed { DANGER } else { VERDIGRIS }, 0.7);
    let rot = if boot {
        cx.ambient = true;
        ((cx.now % 1.1) / 1.1) as f32 * std::f32::consts::TAU
    } else {
        0.0
    };
    let rc = pos2(x + 11.0, y + 18.0);
    w::ring(&p, rc, 9.0, 1.0 - off / 56.6, ring_c, RING_TRACK, 2.0, rot);
    let title = if boot {
        ["Booting kernel", "Starting agent", "Starting Docker Engine"][s.boot.min(2)].to_string()
    } else {
        match eng {
            Engine::Ready => "Ready in 1.7 s".into(),
            Engine::Off => "Engine stopped".into(),
            Engine::Failed => "Engine failed".into(),
            _ => "Engine running".into(),
        }
    };
    let sub = if boot {
        "Starting".to_string()
    } else {
        match eng {
            Engine::Ready => "Resuming containers".into(),
            Engine::Off => "Nothing is running".into(),
            Engine::Failed => "Could not start the VM".into(),
            _ => format!("{} CPU · {} GB · arm64", s.saved.0, s.saved.1),
        }
    };
    w::text(&p, ui, x + 32.0, y + 9.5, &title, medium(13.5), TEXT);
    w::text(&p, ui, x + 32.0, y + 27.5, &sub, sans(12.0), MUTED);
    y += 36.0 + 14.0;

    // Memory
    let used = s.used_gb();
    let mem_txt = if live { format!("{:.1} of {} GB", used, s.saved.1) } else { "—".into() };
    w::text(&p, ui, x, y + 8.0, "Memory", sans(12.0), MUTED);
    w::text_right(&p, ui, right, y + 8.0, &mem_txt, mono(11.5), TEXT2);
    let frac = if live { (used / s.saved.1 as f64) as f32 } else { 0.0 };
    let frac = cx.tw.f("eng-mem", frac, 1.0, Ease::STD);
    w::bar(&p, Rect::from_min_size(pos2(x, y + 24.0), vec2(right - x, 3.0)), frac, VERDIGRIS, TRACK);
    y += 27.0 + 14.0;

    // Buttons
    let btn = |label: &str| Btn::ghost(label).h(28.0).radius(8.0).pad(0.0).font(sans(12.5)).color(TEXT2).border(BTN_EDGE_SOFT);
    let full = right - x;
    if live {
        let half = (full - 8.0) / 2.0;
        if btn("Restart").at(ui, Rect::from_min_size(pos2(x, y), vec2(half, 28.0)), Id::new("eng-restart")).clicked() {
            cx.fx.push(Box::new(|s| s.reboot()));
        }
        if btn("Stop").at(ui, Rect::from_min_size(pos2(x + half + 8.0, y), vec2(half, 28.0)), Id::new("eng-power")).clicked() {
            cx.fx.push(Box::new(|s| s.power_flip()));
        }
    } else {
        let label = if down { "Start" } else { "Stop" };
        if btn(label).at(ui, Rect::from_min_size(pos2(x, y), vec2(full, 28.0)), Id::new("eng-power")).clicked() {
            cx.fx.push(Box::new(|s| s.power_flip()));
        }
    }
}
