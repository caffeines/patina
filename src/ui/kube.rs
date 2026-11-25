//! Kubernetes: the cluster switch and the kube-system pods.

use egui::text::{LayoutJob, TextFormat};
use egui::{pos2, vec2, Align2, Id, Rect, Ui, UiBuilder};

use super::{enter, Cx};
use crate::sim::{Kube, ModalKind, St};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let wd = ui.available_width();
    let origin = ui.cursor().min;
    let view_at = cx.s.view_at;
    let kube = cx.s.kube;
    let k_on = kube == Kube::On;
    let k_busy = kube == Kube::Starting;
    let gap = 24.0;
    let card_w = 400.0f32.min(wd);
    let stacked = wd < card_w + gap + 420.0;
    let list_x = if stacked { origin.x } else { origin.x + card_w + gap };
    let list_w = if stacked { wd } else { wd - card_w - gap };

    // ---- cluster card
    let para_w = card_w - 40.0;
    let mut job = LayoutJob::default();
    let base = TextFormat { font_id: sans(12.5), color: MUTED, line_height: Some(17.5), ..Default::default() };
    job.append("kubectl uses the ", 0.0, base.clone());
    job.append("patina", 0.0, TextFormat { font_id: mono(12.5), color: CODE, line_height: Some(17.5), ..Default::default() });
    job.append(" context. Images on this Mac are available to the cluster without a registry.", 0.0, base);
    job.wrap.max_width = para_w;
    let para = ui.fonts(|f| f.layout_job(job));
    let card_h = 20.0 + 36.0 + 16.0 + para.size().y + if k_on { 16.0 + 30.0 } else { 0.0 } + 20.0;
    let (a, dy) = enter(cx.tw, view_at, 0.12, 0.55);
    let card = Rect::from_min_size(origin + vec2(0.0, dy), vec2(card_w, card_h));
    let mut cui = ui.new_child(UiBuilder::new().max_rect(card));
    cui.multiply_opacity(a);
    let p = cui.painter().clone();
    w::card(&p, card, 14.0, CARD, CARD_EDGE);
    let x = card.left() + 20.0;
    let right = card.right() - 20.0;
    let hy = card.top() + 20.0 + 18.0;

    let off = if k_on {
        0.0
    } else if k_busy {
        [44.0, 28.0, 12.0][cx.s.kstep.min(2)]
    } else {
        56.6
    };
    let off = cx.tw.f("kube-ring", off, 0.5, Ease::STD);
    let ring_c = cx.tw.c("kube-ring-c", if k_busy { COPPER } else { VERDIGRIS }, 0.7);
    let rot = if k_busy {
        cx.ambient = true;
        ((cx.now % 1.1) / 1.1) as f32 * std::f32::consts::TAU
    } else {
        0.0
    };
    w::ring(&p, pos2(x + 11.0, hy), 9.0, 1.0 - off / 56.6, ring_c, RING_TRACK, 2.0, rot);
    let title = if k_on {
        "Cluster running"
    } else if k_busy {
        ["Downloading components", "Starting the control plane", "Scheduling system pods"][cx.s.kstep.min(2)]
    } else {
        "Kubernetes is off"
    };
    let sub = if k_on {
        "Single node · arm64"
    } else if k_busy {
        "This takes a moment the first time"
    } else {
        "Turn it on to run a local cluster"
    };
    let tg = w::g_ls(&cui, title, semibold(15.0), TEXT, -0.15);
    w::put(&p, &tg, pos2(x + 34.0, hy - 9.0), Align2::LEFT_CENTER);
    w::text(&p, &cui, x + 34.0, hy + 10.0, sub, sans(12.5), TEXT2);
    let sw = Rect::from_min_size(pos2(right - 40.0, hy - 12.0), vec2(40.0, 24.0));
    let bg = if k_on {
        VERDIGRIS
    } else if k_busy {
        COPPER
    } else {
        SWITCH_OFF
    };
    if w::switch(&mut cui, cx.tw, sw, Id::new("kube-switch"), kube != Kube::Off, bg).clicked() {
        cx.fx.push(Box::new(|s| s.flip_kube()));
    }
    let py = card.top() + 20.0 + 36.0 + 16.0;
    p.galley(pos2(x, py), para.clone(), MUTED);
    if k_on {
        let by = py + para.size().y + 16.0;
        let (resp, _) = Btn::ghost("Reset cluster").h(30.0).pad(13.0).radius(8.0).at_left(&mut cui, x, by + 15.0, Id::new("kube-reset"));
        if resp.clicked() {
            cx.fx.push(Box::new(|s| s.ask_modal(ModalKind::Reset, "")));
        }
    }

    // ---- pods
    let top = if stacked { origin.y + card_h + gap } else { origin.y };
    let (a2, dy2) = enter(cx.tw, view_at, 0.2, 0.55);
    let pods: Vec<_> = cx.s.pods.iter().filter(|p| p.shown).cloned().collect();
    let pods_running = crate::sim::fmt_count(pods.iter().filter(|p| p.st == St::Running).count(), "pod") + " running";
    let list_h = 37.0 + if kube == Kube::Off { 44.0 } else { pods.len() as f32 * 48.0 };
    let lrect = Rect::from_min_size(pos2(list_x, top + dy2), vec2(list_w, list_h));
    let mut lui = ui.new_child(UiBuilder::new().max_rect(lrect));
    lui.multiply_opacity(a2);
    let lp = lui.painter().clone();
    let cy = lrect.top() + 6.0 + 15.5;
    let lab = w::g_ls(&lui, "kube-system", semibold(12.0), TEXT2, 0.48);
    let lr = w::put(&lp, &lab, pos2(lrect.left() + 14.0, cy), Align2::LEFT_CENTER);
    let meta = if k_on {
        &pods_running
    } else if k_busy {
        "starting"
    } else {
        "off"
    };
    w::text(&lp, &lui, lr.right() + 10.0, cy, meta, sans(12.0), MUTED);
    let mut y = lrect.top() + 37.0;
    if kube == Kube::Off {
        w::text(&lp, &lui, lrect.left() + 14.0, y + 12.0 + 9.5, "System pods appear here when the cluster is on.", sans(13.5), MUTED);
    }
    for pd in &pods {
        let (pa, pdy) = enter(cx.tw, pd.shown_at.max(view_at), 0.0, 0.45);
        let row = Rect::from_min_size(pos2(lrect.left(), y + 1.0 + pdy), vec2(list_w, 46.0));
        let mut rp = lp.clone();
        rp.multiply_opacity(pa);
        let hv = cx.tw.f(("pod-hv", &pd.id), if lui.rect_contains_pointer(row) { 1.0 } else { 0.0 }, 0.2, Ease::EASE);
        if hv > 0.01 {
            w::fill(&rp, row, 10.0, ROW_HOVER.gamma_multiply(hv));
        }
        let rcy = row.center().y;
        cx.ambient |= w::status_dot(&rp, cx.tw, Id::new(("pod", &pd.id)), pos2(row.left() + 14.0 + 5.0, rcy), pd.st, false, pd.fresh_at);
        let running = pd.st == St::Running;
        let name_w = list_w - 28.0 - 22.0 - 14.0 - 44.0 - 14.0 - 130.0 - 14.0;
        let ng = w::g_elide(&lui, &pd.id, mono(12.5), TEXT, name_w);
        w::put(&rp, &ng, pos2(row.left() + 14.0 + 22.0 + 14.0, rcy), Align2::LEFT_CENTER);
        let ready_r = row.right() - 14.0 - 130.0 - 14.0;
        let ready = if !pd.ready.is_empty() { pd.ready.as_str() } else if running { "1/1" } else { "0/1" };
        let phase = if !pd.phase.is_empty() { pd.phase.as_str() } else if running { "Running" } else { "ContainerCreating" };
        w::text_right(&rp, &lui, ready_r, rcy, ready, mono(12.0), TEXT2);
        w::text_right(&rp, &lui, row.right() - 14.0, rcy, phase, sans(12.5), MUTED);
        y += 48.0;
    }

    let total = if stacked { card_h + gap + list_h } else { card_h.max(list_h) };
    ui.advance_cursor_after_rect(Rect::from_min_size(origin, vec2(wd, total)));
}
