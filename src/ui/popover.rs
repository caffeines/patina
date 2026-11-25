//! The menu bar popover (`MenuBar.dc.html`): the engine switch and status,
//! each container with its start/stop button and CPU bars, Open Patina and
//! Stop all. It draws into whatever rect it is given: the popover window
//! under the status item, or the main window with `--popover`.

use std::f32::consts::PI;

use egui::epaint::Mesh;
use egui::{pos2, vec2, Align2, Color32, CursorIcon, Id, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder};

use super::side::engine_text;
use super::{enter, Cx};
use crate::icons::{self, Ink};
use crate::sim::{Banner, Engine, St, View};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

const ROW_H: f32 = 38.0;
const ROW_GAP: f32 = 2.0;
/// Circumference of the 22px status ring, as in the design's dash offsets.
const RING_LEN: f32 = 138.3;

/// What the popover asks of the app.
#[derive(Default)]
pub struct Out {
    /// Bring the main window forward.
    pub open: bool,
    /// Close the popover.
    pub close: bool,
}

pub fn show(ui: &mut Ui, cx: &mut Cx, t0: f64) -> Out {
    let mut out = Out::default();
    let full = ui.max_rect();
    let p = ui.painter().clone();
    w::fill(&p, full, 16.0, SIDEBAR);
    w::edge(&p, full, 16.0, POP_EDGE);
    let r = full.shrink(16.0);

    let s = &*cx.s;
    let eng = s.engine;
    let boot = eng == Engine::Booting;
    let failed = eng == Engine::Failed;
    let down = matches!(eng, Engine::Off | Engine::Failed);
    let setup = s.is_live() && !s.installed && down;
    let note = s.banner == Some(Banner::Update) && !down && !boot;
    let n_run = s.alive().filter(|c| c.st == St::Running).count();
    let any = s.alive().next().is_some();

    // ------------------------------------------------------------ header
    let mut y = r.top();
    {
        let (a, dy) = enter(cx.tw, t0, 0.0, 0.55);
        let row = Rect::from_min_size(pos2(r.left() + 4.0, y + dy), vec2(r.width() - 8.0, 24.0));
        let mut hui = ui.new_child(UiBuilder::new().max_rect(row));
        hui.multiply_opacity(a);
        let hp = hui.painter().clone();
        let cy = row.center().y;
        w::logo(&hp, pos2(row.left() + 10.0, cy), 10.0 * 20.0 / 22.0);
        let gal = w::g_ls(&hui, "Patina", semibold(15.0), TEXT, -0.15);
        w::put(&hp, &gal, pos2(row.left() + 30.0, cy), Align2::LEFT_CENTER);
        let sw = Rect::from_min_size(pos2(row.right() - 40.0, cy - 12.0), vec2(40.0, 24.0));
        w::text_right(&hp, &hui, sw.left() - 10.0, cy, "Engine", sans(12.0), TEXT2);
        let bg = if down {
            SWITCH_OFF
        } else if boot {
            COPPER
        } else {
            VERDIGRIS
        };
        if w::switch(&mut hui, cx.tw, sw, Id::new("mb-switch"), !down, bg).clicked() {
            if setup {
                out.open = true;
            }
            cx.fx.push(Box::new(|s| s.power_flip()));
        }
    }
    y += 24.0 + 14.0;

    // ------------------------------------------------------- engine card
    {
        let (a, dy) = enter(cx.tw, t0, 0.08, 0.55);
        let card = Rect::from_min_size(pos2(r.left(), y + dy), vec2(r.width(), 80.0));
        let mut cui = ui.new_child(UiBuilder::new().max_rect(card));
        cui.multiply_opacity(a);
        let cp = cui.painter().clone();
        let bd = cx.tw.c("mb-hero-bd", if failed { ERR_EDGE } else if note { WARN_EDGE } else { CARD_EDGE }, 0.5);
        w::card(&cp, card, 14.0, ENGINE_CARD, bd);

        let rc = pos2(card.left() + 14.0 + 26.0, card.center().y);
        if eng == Engine::Running && n_run > 0 {
            // @keyframes glow: the ring's halo swells and fades every 3 s.
            let ph = ((cx.now % 3.0) / 3.0) as f32;
            if ph < 0.7 {
                let e = Ease::EASE_OUT.at(ph / 0.7);
                let k = 1.0 + 0.38 * e;
                cp.circle_stroke(rc, 25.25 * k, Stroke::new(1.5 * k, VERDIGRIS.gamma_multiply(0.6 * (1.0 - e))));
            }
            cx.ambient = true;
        }
        let ring_off = if setup {
            RING_LEN
        } else if failed {
            0.0
        } else if down {
            RING_LEN
        } else if boot {
            [110.0, 72.0, 30.0][s.boot.min(2)]
        } else {
            0.0
        };
        let off = cx.tw.f("mb-ring", ring_off, 0.55, Ease::STD);
        let ring_c = cx.tw.c("mb-ring-c", if boot { COPPER } else if failed { DANGER } else { VERDIGRIS }, 0.7);
        w::ring(&cp, rc, 22.0, 1.0 - off / RING_LEN, ring_c, RING_TRACK, 2.5, 0.0);
        let count = w::g(&cui, &n_run.to_string(), mono(17.0), TEXT);
        w::put(&cp, &count, rc, Align2::CENTER_CENTER);

        let (title, sub) = if setup {
            ("Patina isn't set up yet".to_string(), "Setup takes a few minutes".to_string())
        } else if note {
            ("Engine running".to_string(), "Version 1.4 is ready".to_string())
        } else {
            engine_text(s, format!("{} running · {:.1} GB memory", n_run, s.used_gb()))
        };
        let hero = if setup {
            Some("Set up")
        } else if failed {
            Some("Try again")
        } else if note {
            Some("Update")
        } else {
            None
        };
        let tx = card.left() + 14.0 + 52.0 + 14.0;
        let mut text_r = card.right() - 14.0;
        if let Some(label) = hero {
            let btn = Btn::ghost(label).h(30.0).pad(12.0).radius(8.0).font(medium(13.5));
            let (resp, br) = btn.at_right(&mut cui, card.right() - 14.0, card.center().y, Id::new("mb-hero"));
            text_r = br.left() - 12.0;
            if resp.clicked() {
                if setup {
                    out.open = true;
                    cx.fx.push(Box::new(|s| s.want_setup = true));
                } else if failed {
                    cx.fx.push(Box::new(|s| s.power_flip()));
                } else {
                    cx.fx.push(Box::new(|s| s.apply_update()));
                }
            }
        }
        let tw_max = (text_r - tx).max(40.0);
        let tg = w::g_elide(&cui, &title, semibold(16.0), TEXT, tw_max);
        let sg = w::g_elide(&cui, &sub, sans(12.5), TEXT2, tw_max);
        let cy = card.center().y;
        w::put(&cp, &tg, pos2(tx, cy - 10.0), Align2::LEFT_CENTER);
        w::put(&cp, &sg, pos2(tx, cy + 11.5), Align2::LEFT_CENTER);
    }
    y += 80.0 + 14.0;

    // ------------------------------------------------------------ footer
    let foot = Rect::from_min_max(pos2(r.left(), r.bottom() - 36.0), r.max);
    {
        let (a, dy) = enter(cx.tw, t0, 0.42, 0.55);
        let foot = foot.translate(vec2(0.0, dy));
        let mut fui = ui.new_child(UiBuilder::new().max_rect(foot.expand(4.0)));
        fui.multiply_opacity(a);
        let mut open_r = foot;
        if any {
            let stop = Btn::ghost("Stop all").h(36.0).pad(14.0).radius(10.0).border(BTN_EDGE).enabled(eng == Engine::Running && n_run > 0);
            let (resp, sr) = stop.at_right(&mut fui, foot.right(), foot.center().y, Id::new("mb-stop-all"));
            open_r.max.x = sr.left() - 10.0;
            if resp.clicked() {
                cx.fx.push(Box::new(|s| s.stop_all()));
            }
        }
        let open = Btn::cta("Open Patina").h(36.0).radius(10.0).sheen(true);
        if open.at(&mut fui, open_r, Id::new("mb-open")).clicked() {
            out.open = true;
        }
        if a > 0.0 {
            cx.ambient = true; // the sheen
        }
    }

    // -------------------------------------------------------- containers
    let list = Rect::from_min_max(pos2(r.left(), y), pos2(r.right(), foot.top() - 14.0));
    if !any {
        let (a, dy) = enter(cx.tw, t0, 0.14, 0.5);
        let mut eui = ui.new_child(UiBuilder::new().max_rect(list));
        eui.multiply_opacity(a);
        let ep = eui.painter().clone();
        let cy = list.center().y + dy;
        let head = w::g_ls(&eui, "No containers yet", semibold(15.0), TEXT, -0.15);
        w::put(&ep, &head, pos2(list.center().x, cy - 12.0), Align2::CENTER_CENTER);
        let mut job = w::job("Run an image from the main window and it shows up here.", sans(13.5), TEXT2, 0.0);
        job.sections[0].format.line_height = Some(19.0);
        job.wrap.max_width = list.width() - 48.0;
        job.halign = egui::Align::Center;
        let body = eui.fonts(|f| f.layout_job(job));
        // Centered rows are laid out around x = 0.
        ep.galley(pos2(list.center().x, cy + 6.0), body, TEXT2);
        return out;
    }

    {
        let (a, dy) = enter(cx.tw, t0, 0.14, 0.55);
        let mut lp = ui.painter().clone();
        lp.multiply_opacity(a);
        let gal = w::g_ls(ui, "Containers", semibold(12.0), TEXT2, 0.48);
        w::put(&lp, &gal, pos2(list.left() + 10.0, list.top() + 8.0 + dy), Align2::LEFT_CENTER);
    }
    let rows = Rect::from_min_max(pos2(list.left(), list.top() + 23.0), list.max);
    let ids: Vec<String> = s.alive().map(|c| c.id.clone()).collect();
    let mut rui = ui.new_child(UiBuilder::new().max_rect(rows));
    let scroll = ScrollArea::vertical().id_salt("mb-rows").auto_shrink([false, false]).show(&mut rui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, ROW_GAP);
        for (i, id) in ids.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(vec2(rows.width(), ROW_H), Sense::hover());
            let (a, dy) = enter(cx.tw, t0, 0.2 + i as f32 * 0.05, 0.5);
            let mut row = ui.new_child(UiBuilder::new().max_rect(rect.translate(vec2(0.0, dy))));
            row.multiply_opacity(a);
            row_ui(&mut row, cx, id, &mut out);
        }
    });
    // Fade the list's edges while there is more to scroll to.
    let (inner, top) = (scroll.inner_rect, scroll.state.offset.y);
    if top > 0.5 {
        fade(&p, Rect::from_min_size(inner.min, vec2(inner.width(), 18.0)), SIDEBAR, Color32::TRANSPARENT);
    }
    if top + inner.height() < scroll.content_size.y - 0.5 {
        fade(&p, Rect::from_min_max(pos2(inner.left(), inner.bottom() - 18.0), inner.max), Color32::TRANSPARENT, SIDEBAR);
    }
    out
}

/// A vertical gradient from `top` to `bottom`.
fn fade(p: &egui::Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut m = Mesh::default();
    m.colored_vertex(r.left_top(), top);
    m.colored_vertex(r.right_top(), top);
    m.colored_vertex(r.left_bottom(), bottom);
    m.colored_vertex(r.right_bottom(), bottom);
    m.add_triangle(0, 1, 2);
    m.add_triangle(1, 3, 2);
    p.add(m);
}

fn row_ui(ui: &mut Ui, cx: &mut Cx, id: &str, out: &mut Out) {
    let Some(c) = cx.s.alive().find(|c| c.id == id).cloned() else { return };
    let rect = ui.max_rect();
    let p = ui.painter().clone();
    let tog_r = Rect::from_min_size(pos2(rect.right() - 6.0 - 28.0, rect.center().y - 14.0), vec2(28.0, 28.0));
    let pick = ui.interact(Rect::from_min_max(rect.min, pos2(tog_r.left() - 2.0, rect.bottom())), Id::new(("mb-row", id)), Sense::click());
    let hov = ui.rect_contains_pointer(rect);
    let ht = ui.ctx().animate_bool_with_time(Id::new(("mb-row-h", id)), hov, 0.2);
    if ht > 0.001 {
        w::fill(&p, rect, 9.0, POP_ROW_HOVER.gamma_multiply(ht));
    }
    if pick.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if pick.clicked() {
        out.open = true;
        let id = id.to_string();
        cx.fx.push(Box::new(move |s| {
            s.go_view(View::Containers);
            s.pick(&id);
        }));
    }

    let cy = rect.center().y;
    let x = rect.left() + 12.0;
    cx.ambient |= w::status_dot(&p, cx.tw, Id::new(("mb-dot", id)), pos2(x + 5.0, cy), c.st, c.bad, c.fresh_at);

    let run = c.st == St::Running;
    let warm = c.st == St::Starting;
    let busy = warm || c.st == St::Stopping;
    let stoppable = run || c.st == St::Paused || c.loops > 0;

    // CPU bars: the newest 8 samples, 2px wide, 16px tall.
    let bars_r = tog_r.left() - 12.0;
    let bars_l = bars_r - (8.0 * 2.0 + 7.0 * 2.0);
    let bar_c = if run {
        if c.bad {
            COPPER
        } else {
            VERDIGRIS
        }
    } else if c.st == St::Paused {
        BAR_PAUSED
    } else if warm {
        BAR_WARM
    } else {
        BAR_IDLE
    };
    let bar_c = cx.tw.c(("mb-bar-c", id), bar_c, 0.6);
    let n = c.bars.len();
    for i in 0..8 {
        let v = cx.tw.f(("mb-bar", id, i), c.bars[n - 8 + i], 1.0, Ease::STD);
        let h = 16.0 * v;
        let bx = bars_l + i as f32 * 4.0;
        p.rect_filled(Rect::from_min_max(pos2(bx, cy + 8.0 - h), pos2(bx + 2.0, cy + 8.0)), 1.0, bar_c);
    }

    let mut name_r = bars_l - 12.0;
    if !c.port.is_empty() {
        let pr = w::text_right(&p, ui, name_r, cy, &format!(":{}", c.port), mono(12.0), MUTED);
        name_r = pr.left() - 12.0;
    }
    let name_x = x + 10.0 + 12.0;
    let name_c = cx.tw.c(("mb-name-c", id), if c.st == St::Stopped { TEXT2 } else { TEXT }, 0.4);
    let name = w::g_elide(ui, &c.id, medium(13.5), name_c, (name_r - name_x).max(30.0));
    w::put(&p, &name, pos2(name_x, cy), Align2::LEFT_CENTER);

    let label = if stoppable { format!("Stop {id}") } else { format!("Start {id}") };
    let tog = if busy && !stoppable {
        let resp = Btn::plain("").w(28.0).h(28.0).at(ui, tog_r, Id::new(("mb-tog", id)));
        let rot = ((cx.now % 0.8) / 0.8) as f32 * std::f32::consts::TAU;
        w::arc(&p, tog_r.center(), 5.5, rot, rot + 1.5 * PI, Stroke::new(1.6_f32, TEXT2));
        cx.ambient = true;
        resp
    } else {
        let ico = if stoppable { icons::STOP } else { icons::PLAY };
        Btn::icon(ico, Ink::Fill).w(28.0).h(28.0).at(ui, tog_r, Id::new(("mb-tog", id)))
    };
    let tog = tog.on_hover_text(label);
    if tog.clicked() {
        let id = id.to_string();
        cx.fx.push(Box::new(move |s| s.toggle(&id)));
    }
}
