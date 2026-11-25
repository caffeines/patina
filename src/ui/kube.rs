//! Kubernetes: the cluster switch with the node's CPU and memory, and a
//! `kubectl get`-style browser of pods, workloads, services, storage and
//! config. Clicking a pod opens its logs.

use egui::text::{LayoutJob, TextFormat};
use egui::{pos2, vec2, Align2, CursorIcon, Id, Rect, Sense, Ui, UiBuilder};

use super::{enter, Cx};
use crate::engine::kube::{fmt_bytes, fmt_cpu, Health, Obj};
use crate::sim::{KTab, Kube, ModalKind, St};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

const SYSTEM: [&str; 3] = ["kube-system", "kube-public", "kube-node-lease"];

/// The tab's columns after NAME, with their widths. AGE comes last.
fn columns(tab: KTab) -> &'static [(&'static str, f32)] {
    match tab {
        KTab::Pods => &[("READY", 44.0), ("STATUS", 118.0), ("RESTARTS", 66.0), ("CPU", 46.0), ("MEMORY", 60.0)],
        KTab::Workloads => &[("KIND", 104.0), ("READY", 96.0)],
        KTab::Services => &[("TYPE", 88.0), ("CLUSTER-IP", 112.0), ("PORTS", 168.0)],
        KTab::Storage => &[("KIND", 44.0), ("STATUS", 80.0), ("CAPACITY", 74.0), ("CLASS", 92.0)],
        KTab::Config => &[("KIND", 88.0), ("DATA", 110.0)],
    }
}

fn dot(h: Health) -> St {
    match h {
        Health::Good => St::Running,
        Health::Busy => St::Starting,
        Health::Bad => St::Error,
        Health::Idle => St::Stopped,
    }
}

/// kubectl's ages: 45s, 12m, 5h, 3d.
fn age(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// How pods are keyed for logs: by name in kube-system, else `ns/name`.
fn pod_key(o: &Obj) -> String {
    if o.ns == "kube-system" {
        o.name.clone()
    } else {
        format!("{}/{}", o.ns, o.name)
    }
}

/// Rows for the tab, filtered by the search: yours first, then the
/// system's. Before the API answers, pods come from their containers.
fn rows(cx: &Cx, tab: KTab) -> Vec<Obj> {
    let q = cx.s.q.trim().to_lowercase();
    let mut out: Vec<Obj> = match &cx.s.ksnap {
        Some(snap) => snap.objs.iter().filter(|o| tab.holds(o.kind)).cloned().collect(),
        None if tab == KTab::Pods => cx
            .s
            .pods
            .iter()
            .filter(|p| p.shown)
            .map(|p| {
                let (ns, name) = p.id.split_once('/').unwrap_or(("kube-system", &p.id));
                let health = match p.st {
                    St::Running => Health::Good,
                    St::Error => Health::Bad,
                    _ => Health::Busy,
                };
                let ready = if p.ready.is_empty() { "—".to_string() } else { p.ready.clone() };
                let phase = if p.phase.is_empty() { if p.st == St::Running { "Running" } else { "ContainerCreating" }.to_string() } else { p.phase.clone() };
                Obj { kind: "Pod", ns: ns.into(), name: name.into(), health, cols: vec![ready, phase, "—".into(), "—".into(), "—".into()], created: 0 }
            })
            .collect(),
        None => Vec::new(),
    };
    out.retain(|o| q.is_empty() || o.name.to_lowercase().contains(&q) || o.ns.to_lowercase().contains(&q));
    out.sort_by(|a, b| (SYSTEM.contains(&a.ns.as_str()), &a.ns, &a.name).cmp(&(SYSTEM.contains(&b.ns.as_str()), &b.ns, &b.name)));
    out
}

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let wd = ui.available_width();
    let origin = ui.cursor().min;
    let gap = 24.0;
    let card_w = 330.0f32.min(wd);
    let stacked = wd < card_w + gap + 560.0;
    let list_x = if stacked { origin.x } else { origin.x + card_w + gap };
    let list_w = if stacked { wd } else { wd - card_w - gap };
    let card_h = card(ui, cx, Rect::from_min_size(origin, vec2(card_w, 0.0)));
    let top = if stacked { origin.y + card_h + gap } else { origin.y };
    let list_h = table(ui, cx, pos2(list_x, top), list_w);
    let total = if stacked { card_h + gap + list_h } else { card_h.max(list_h) };
    ui.advance_cursor_after_rect(Rect::from_min_size(origin, vec2(wd, total)));
}

/// The cluster card. Returns its height.
fn card(ui: &mut Ui, cx: &mut Cx, at: Rect) -> f32 {
    let kube = cx.s.kube;
    let k_on = kube == Kube::On;
    let k_busy = kube == Kube::Starting;
    let node = cx.s.ksnap.as_ref().and_then(|s| s.node.clone()).filter(|_| k_on);
    let card_w = at.width();
    let para_w = card_w - 40.0;
    let mut job = LayoutJob::default();
    let base = TextFormat { font_id: sans(12.5), color: MUTED, line_height: Some(17.5), ..Default::default() };
    job.append("kubectl uses the ", 0.0, base.clone());
    job.append("patina", 0.0, TextFormat { font_id: mono(12.5), color: CODE, line_height: Some(17.5), ..Default::default() });
    job.append(" context. Images on this Mac are available to the cluster without a registry.", 0.0, base);
    job.wrap.max_width = para_w;
    let para = ui.fonts(|f| f.layout_job(job));
    let stats_h = if node.is_some() { 18.0 + 62.0 } else { 0.0 };
    let card_h = 20.0 + 36.0 + 16.0 + para.size().y + stats_h + if k_on { 18.0 + 30.0 } else { 0.0 } + 20.0;
    let (a, dy) = enter(cx.tw, cx.s.view_at, 0.12, 0.55);
    let card = Rect::from_min_size(at.min + vec2(0.0, dy), vec2(card_w, card_h));
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
    let arch = if cx.s.info.arch.is_empty() { "arm64".to_string() } else { cx.s.info.arch.clone() };
    let sub = if k_on {
        match &node {
            Some(n) if !n.version.is_empty() => format!("Single node · {arch} · {}", n.version),
            _ => format!("Single node · {arch}"),
        }
    } else if k_busy {
        "This takes a moment the first time".into()
    } else {
        "Turn it on to run a local cluster".into()
    };
    let tg = w::g_ls(&cui, title, semibold(15.0), TEXT, -0.15);
    w::put(&p, &tg, pos2(x + 34.0, hy - 9.0), Align2::LEFT_CENTER);
    let sg = w::g_elide(&cui, &sub, sans(12.5), TEXT2, right - 52.0 - x - 34.0);
    w::put(&p, &sg, pos2(x + 34.0, hy + 10.0), Align2::LEFT_CENTER);
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
    let mut y = card.top() + 20.0 + 36.0 + 16.0;
    p.galley(pos2(x, y), para.clone(), MUTED);
    y += para.size().y;

    // The node's CPU and memory, from metrics-server.
    if let Some(n) = &node {
        y += 18.0;
        let col_w = (para_w - 16.0) / 2.0;
        let cpu = n.cpu_m.map_or("—".to_string(), fmt_cpu);
        let mem = n.mem.map_or("—".to_string(), fmt_bytes);
        let stats = [
            ("CPU", cpu, format!("of {}", fmt_cpu(n.cpu_cap_m)), n.cpu_m.map_or(0.0, |c| (c / n.cpu_cap_m.max(1.0)) as f32)),
            ("Memory", mem, format!("of {}", fmt_bytes(n.mem_cap)), n.mem.map_or(0.0, |m| (m / n.mem_cap.max(1.0)) as f32)),
        ];
        for (i, (label, val, cap, f)) in stats.into_iter().enumerate() {
            let cx0 = x + i as f32 * (col_w + 16.0);
            w::text(&p, &cui, cx0, y + 8.5, label, sans(12.0), MUTED);
            w::text_right(&p, &cui, cx0 + col_w, y + 8.5, &cap, mono(11.5), MUTED);
            let vg = w::g_ls(&cui, &val, mono(22.0), TEXT, -0.66);
            w::put(&p, &vg, pos2(cx0, y + 38.0), Align2::LEFT_CENTER);
            let fv = cx.tw.f(("kube-bar", i), f.min(1.0), 1.0, Ease::STD);
            w::bar(&p, Rect::from_min_size(pos2(cx0, y + 59.0), vec2(col_w, 3.0)), fv, VERDIGRIS, TRACK);
        }
        y += 62.0;
    }
    if k_on {
        y += 18.0;
        let (resp, _) = Btn::ghost("Reset cluster").h(30.0).pad(13.0).radius(8.0).at_left(&mut cui, x, y + 15.0, Id::new("kube-reset"));
        if resp.clicked() {
            cx.fx.push(Box::new(|s| s.ask_modal(ModalKind::Reset, "")));
        }
    }
    card_h
}

/// The resource table for the selected tab. Returns its height.
fn table(ui: &mut Ui, cx: &mut Cx, at: egui::Pos2, wd: f32) -> f32 {
    let kube = cx.s.kube;
    let tab = if kube == Kube::On { cx.s.ktab } else { KTab::Pods };
    let (a, dy) = enter(cx.tw, cx.s.view_at, 0.2, 0.55);
    let now = chrono::Utc::now().timestamp();
    let list = rows(cx, tab);
    let cols = columns(tab);
    let head_h = 30.0;
    let row_h = 50.0;
    let note = if kube == Kube::Off {
        Some("Pods appear here when the cluster is on.".to_string())
    } else if list.is_empty() && cx.s.ksnap.is_none() && cx.s.info.stale && kube == Kube::On {
        Some("The engine is from an older Patina and doesn't forward the cluster's API. Restart it to reach the cluster.".to_string())
    } else if list.is_empty() && cx.s.ksnap.is_none() && !cx.s.kerr.is_empty() && kube == Kube::On {
        Some(cx.s.kerr.clone())
    } else if list.is_empty() && !cx.s.q.trim().is_empty() {
        Some(format!("No {} match “{}”.", tab.label().to_lowercase(), cx.s.q.trim()))
    } else if list.is_empty() && kube == Kube::On {
        Some(format!("No {} yet.", tab.label().to_lowercase()))
    } else {
        None
    };
    let h = head_h + if note.is_some() { 44.0 } else { list.len() as f32 * row_h };
    let rect = Rect::from_min_size(at + vec2(0.0, dy), vec2(wd, h));
    let mut tui = ui.new_child(UiBuilder::new().max_rect(rect));
    tui.multiply_opacity(a);
    let p = tui.painter().clone();

    // Columns: dot 22 | name (flex) | the tab's | age 52
    let x = rect.left() + 14.0;
    let age_r = rect.right() - 14.0;
    let fixed: f32 = cols.iter().map(|c| c.1 + 12.0).sum::<f32>() + 40.0;
    let name_x = x + 22.0 + 12.0;
    let name_w = (age_r - fixed - name_x).max(120.0);
    let col_x: Vec<f32> = cols.iter().scan(name_x + name_w + 12.0, |cx0, c| {
        let here = *cx0;
        *cx0 += c.1 + 12.0;
        Some(here)
    }).collect();

    let hy = rect.top() + head_h / 2.0;
    let label = |t: &str| w::g_ls(&tui, t, semibold(11.0), MUTED, 0.66);
    w::put(&p, &label("NAME"), pos2(name_x, hy), Align2::LEFT_CENTER);
    for ((t, _), cx0) in cols.iter().zip(&col_x) {
        w::put(&p, &label(t), pos2(*cx0, hy), Align2::LEFT_CENTER);
    }
    w::put(&p, &label("AGE"), pos2(age_r, hy), Align2::RIGHT_CENTER);
    p.line_segment([pos2(rect.left(), rect.top() + head_h - 0.5), pos2(rect.right(), rect.top() + head_h - 0.5)], egui::Stroke::new(1.0_f32, CARD_EDGE));

    if let Some(n) = note {
        let col = if cx.s.kerr == n { DANGER_TEXT } else { MUTED };
        let g = w::g_elide(&tui, &n, sans(13.5), col, wd - 28.0);
        w::put(&p, &g, pos2(x, rect.top() + head_h + 22.0), Align2::LEFT_CENTER);
        return h + dy;
    }
    let mut y = rect.top() + head_h;
    for o in &list {
        let row = Rect::from_min_size(pos2(rect.left(), y + 2.0), vec2(wd, row_h - 4.0));
        let key = pod_key(o);
        let logs = (o.kind == "Pod").then(|| cx.s.pod_logs.get(&key).cloned()).flatten();
        let resp = tui.interact(row, Id::new(("kobj", o.kind, &o.ns, &o.name)), if logs.is_some() { Sense::click() } else { Sense::hover() });
        let hv = cx.tw.f(("kobj-hv", o.kind, &o.ns, &o.name), if resp.hovered() { 1.0 } else { 0.0 }, 0.2, Ease::EASE);
        if hv > 0.01 {
            w::fill(&p, row, 10.0, ROW_HOVER.gamma_multiply(hv));
        }
        if let Some(container) = logs {
            if resp.hovered() {
                tui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if resp.clicked() {
                cx.fx.push(Box::new(move |s| s.open_logs(&container)));
            }
        }
        let cy = row.center().y;
        cx.ambient |= w::status_dot(&p, cx.tw, Id::new(("kdot", o.kind, &o.ns, &o.name)), pos2(x + 5.0, cy), dot(o.health), false, -10.0);
        let ng = w::g_elide(&tui, &o.name, medium(13.5), TEXT, name_w - 8.0);
        w::put(&p, &ng, pos2(name_x, cy - 8.0), Align2::LEFT_CENTER);
        let ns = if o.ns.is_empty() { "cluster" } else { &o.ns };
        let sg = w::g_elide(&tui, ns, mono(11.5), MUTED, name_w - 8.0);
        w::put(&p, &sg, pos2(name_x, cy + 9.5), Align2::LEFT_CENTER);
        for (i, ((_, cw), cx0)) in cols.iter().zip(&col_x).enumerate() {
            let v = o.cols.get(i).map(String::as_str).unwrap_or("");
            // A pod's status takes its health's colour.
            let col = if tab == KTab::Pods && i == 1 {
                match o.health {
                    Health::Bad => DANGER_TEXT,
                    Health::Busy => COPPER_TEXT,
                    _ => TEXT2,
                }
            } else {
                TEXT2
            };
            let g = w::g_elide(&tui, v, mono(12.0), col, *cw);
            w::put(&p, &g, pos2(*cx0, cy), Align2::LEFT_CENTER);
        }
        let ag = if o.created > 0 { age(now - o.created) } else { "—".into() };
        w::text_right(&p, &tui, age_r, cy, &ag, mono(12.0), TEXT2);
        y += row_h;
    }
    h + dy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_like_kubectl() {
        assert_eq!(age(42), "42s");
        assert_eq!(age(12 * 60), "12m");
        assert_eq!(age(5 * 3600 + 59), "5h");
        assert_eq!(age(3 * 86_400), "3d");
    }
}
