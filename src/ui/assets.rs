//! Images, volumes and networks: one table, plus the image pull progress.

use egui::{pos2, vec2, Align2, Id, Rect, Ui, UiBuilder};

use super::{enter, Cx};
use crate::icons::{self, Ink};
use crate::sim::{fmt_size, Kind, St, View};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

struct Row {
    n: String,
    frac: f32,
    c2: String,
    c3: String,
    c3_color: egui::Color32,
    dot: egui::Color32,
    c4: String,
    deletable: bool,
    seen: bool,
    fresh: bool,
    born: f64,
}

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let view = cx.s.view;
    if view == View::Images && cx.s.pull.is_some() {
        pull_card(ui, cx);
        ui.add_space(22.0);
    }
    let q = cx.s.q.trim().to_lowercase();
    let hit = |n: &str| q.is_empty() || n.to_lowercase().contains(&q);
    let boxes: Vec<(String, String, St)> = cx.s.alive().map(|c| (c.id.clone(), c.grp.clone(), c.st)).collect();
    let kind = match view {
        View::Volumes => Kind::Vols,
        View::Networks => Kind::Nets,
        _ => Kind::Imgs,
    };
    let rows: Vec<Row> = if kind == Kind::Nets {
        cx.s.nets
            .iter()
            .map(|a| {
                let att = a.attached.unwrap_or_else(|| if a.grp.is_empty() { 0 } else { boxes.iter().filter(|b| b.1 == a.grp).count() });
                Row {
                    n: a.n.clone(),
                    frac: att as f32 / boxes.len().max(1) as f32,
                    c2: att.to_string(),
                    c3: if a.subnet.is_empty() { "no subnet".into() } else { a.subnet.clone() },
                    c3_color: if a.subnet.is_empty() { MUTED } else { CODE },
                    dot: egui::Color32::TRANSPARENT,
                    c4: a.driver.clone(),
                    deletable: !a.sys,
                    seen: !a.gone && hit(&a.n),
                    fresh: a.fresh,
                    born: a.born,
                }
            })
            .collect()
    } else {
        let list = if kind == Kind::Vols { &cx.s.vols } else { &cx.s.imgs };
        let max_mb = list.iter().filter(|a| !a.gone).map(|a| a.mb).fold(1.0, f64::max);
        list.iter()
            .map(|a| {
                let u = boxes.iter().find(|b| b.0 == a.used_by);
                Row {
                    n: a.n.clone(),
                    frac: ((a.mb / max_mb) as f32).max(0.02),
                    c2: fmt_size(a.mb),
                    c3: u.map_or("unused".into(), |b| b.0.clone()),
                    c3_color: if u.is_some() { TEXT } else { MUTED },
                    dot: match u.map(|b| b.2) {
                        None => egui::Color32::TRANSPARENT,
                        Some(St::Running) => VERDIGRIS,
                        Some(St::Starting) => COPPER,
                        Some(St::Error) => DANGER,
                        Some(_) => DIM,
                    },
                    c4: a.age.clone(),
                    deletable: true,
                    seen: !a.gone && hit(&a.n),
                    fresh: a.fresh,
                    born: a.born,
                }
            })
            .collect()
    };

    let wd = ui.available_width();
    let origin = ui.cursor().min;
    // grid: 1.3fr | minmax(80,170) | 64 | 1fr | 96 | 30, gap 18, padding 0 8 0 14
    let inner = wd - 22.0 - 5.0 * 18.0 - 64.0 - 96.0 - 30.0;
    let bar_w = (inner * 0.25).clamp(80.0, 170.0);
    let c1_w = (inner - bar_w) * 1.3 / 2.3;
    let c3_w = (inner - bar_w) / 2.3;
    let view_at = cx.s.view_at;
    let mut y = origin.y;
    for (i, r) in rows.iter().enumerate() {
        let rh = cx.tw.f(("arow", view.slug(), &r.n), if r.seen { 46.0 } else { 0.0 }, 0.4, Ease::OUT);
        let ro = cx.tw.f(("arow-o", view.slug(), &r.n), if r.seen { 1.0 } else { 0.0 }, 0.3, Ease::EASE);
        if rh > 0.5 {
            let delay = if r.fresh { 0.0 } else { 0.12 + i as f32 * 0.035 };
            let t0 = view_at.max(r.born);
            let (a, dy) = enter(cx.tw, t0, delay, 0.5);
            let clip = Rect::from_min_size(pos2(origin.x, y), vec2(wd, rh));
            let mut rui = ui.new_child(UiBuilder::new().max_rect(clip));
            rui.set_clip_rect(clip.intersect(ui.clip_rect()));
            rui.multiply_opacity(a * ro);
            let p = rui.painter().clone();
            let row = Rect::from_min_size(pos2(origin.x, y + 1.0 + dy), vec2(wd, 44.0));
            let hv = cx.tw.f(("arow-hv", &r.n), if r.seen && rui.rect_contains_pointer(row) { 1.0 } else { 0.0 }, 0.2, Ease::EASE);
            if r.fresh {
                w::card(&p, row, 10.0, ROW_SEL, ROW_SEL_EDGE);
            } else if hv > 0.01 {
                w::fill(&p, row, 10.0, ROW_HOVER.gamma_multiply(hv));
            }
            let cy = row.center().y;
            let mut x = row.left() + 14.0;
            let g1 = w::g_elide_start(&rui, &r.n, mono(12.5), TEXT, c1_w);
            w::put(&p, &g1, pos2(x, cy), Align2::LEFT_CENTER);
            x += c1_w + 18.0;
            // size bar grows in from the left
            let grow = {
                let (ga, _, running) = crate::tween::rise(cx.now, t0, delay, 0.9, 0.0);
                if running {
                    cx.tw.busy = true;
                }
                ga
            };
            let fw = cx.tw.f(("abar", view.slug(), &r.n), r.frac, 0.6, Ease::OUT);
            w::bar(&p, Rect::from_min_size(pos2(x, cy - 1.5), vec2(bar_w, 3.0)), fw * grow, DIM, ASSET_TRACK);
            x += bar_w + 18.0;
            w::text_right(&p, &rui, x + 64.0, cy, &r.c2, mono(12.0), TEXT);
            x += 64.0 + 18.0;
            let dot = cx.tw.c(("adot", &r.n), r.dot, 0.6);
            w::small_dot(&p, pos2(x + 3.5, cy), dot);
            let g3 = w::g_elide(&rui, &r.c3, sans(13.5), r.c3_color, c3_w - 15.0);
            w::put(&p, &g3, pos2(x + 15.0, cy), Align2::LEFT_CENTER);
            x += c3_w + 18.0;
            w::text_right(&p, &rui, x + 96.0, cy, &r.c4, sans(12.5), MUTED);
            x += 96.0 + 18.0;
            if r.deletable {
                let b = Btn::icon(icons::TRASH, Ink::Stroke(1.3)).color(MUTED);
                let resp = b.at(&mut rui, Rect::from_min_size(pos2(x, cy - 15.0), vec2(30.0, 30.0)), Id::new(("adel", view.slug(), &r.n)));
                if resp.clicked() && r.seen {
                    let n = r.n.clone();
                    cx.fx.push(Box::new(move |s| s.ask_asset(kind, &n)));
                }
            }
        }
        y += rh;
    }
    ui.advance_cursor_after_rect(Rect::from_min_max(origin, pos2(origin.x + wd, y)));
}

fn pull_card(ui: &mut Ui, cx: &mut Cx) {
    let Some(pull) = cx.s.pull.clone() else { return };
    let wd = ui.available_width();
    let bad = pull.err;
    let end = pull.done;
    let x0 = ui.cursor().min.x + 20.0;
    let iw = wd - 40.0;
    let err_g = (bad && !pull.msg.is_empty()).then(|| w::g_wrap(ui, &pull.msg, sans(12.5), DANGER_TEXT, iw, 17.5));
    let rows = pull.layers.len().div_ceil(4);
    let layers_h = if rows == 0 { 0.0 } else { rows as f32 * 27.0 + (rows - 1) as f32 * 14.0 + 16.0 };
    let err_h = err_g.as_ref().map_or(0.0, |g| g.size().y + 16.0);
    let h = 18.0 + 28.0 + err_h + layers_h + 18.0;
    let (rect, _) = ui.allocate_exact_size(vec2(wd, h), egui::Sense::hover());
    let p = ui.painter().clone();
    w::card(&p, rect, 14.0, CARD, CARD_EDGE);
    let x = x0;
    let right = rect.right() - 20.0;
    let mut y = rect.top() + 18.0;
    let cy = y + 14.0;

    // status mark
    let mc = pos2(x + 7.0, cy);
    if bad {
        p.circle_filled(mc, 7.0, DANGER);
    } else if end {
        p.circle_filled(mc, 7.0, VERDIGRIS);
    } else {
        w::spin_ring(&p, mc, 6.25, 1.5, rgba(212, 145, 92, 0.25), COPPER, cx.now, 0.8);
        cx.ambient = true;
    }
    let verb = if bad {
        "Could not pull"
    } else if end {
        "Pulled"
    } else if pull.layers.is_empty() {
        "Resolving"
    } else {
        "Pulling"
    };
    let title = format!("{verb} {}", pull.image);
    let mut r_edge = right;
    if bad {
        let (resp, br) = Btn::ghost("Try again").h(28.0).pad(12.0).radius(8.0).at_right(ui, right, cy, Id::new("pull-retry"));
        if resp.clicked() {
            cx.fx.push(Box::new(|s| s.retry_pull()));
        }
        r_edge = br.left() - 12.0;
    } else if !pull.layers.is_empty() {
        let pct = (pull.layers.iter().map(|l| l.1).sum::<f64>() / pull.layers.len() as f64 * 100.0).round() as i64;
        let r = w::text_right(&p, ui, r_edge, cy, &format!("{pct}%"), mono(12.0), TEXT2);
        r_edge = r.left() - 12.0;
    }
    let tg = w::g_elide(ui, &title, mono(12.5), TEXT, r_edge - (x + 26.0));
    w::put(&p, &tg, pos2(x + 26.0, cy), Align2::LEFT_CENTER);
    y += 28.0;
    if let Some(g) = err_g {
        y += 16.0;
        p.galley(pos2(x, y), g.clone(), DANGER_TEXT);
        y += g.size().y;
    }
    if rows > 0 {
        y += 16.0;
    }
    let col_w = (right - x - 3.0 * 14.0) / 4.0;
    for (i, (id, v, size)) in pull.layers.iter().enumerate() {
        let lx = x + (i % 4) as f32 * (col_w + 14.0);
        let ly = y + (i / 4) as f32 * (27.0 + 14.0);
        let col = if *v >= 1.0 { VERDIGRIS } else if bad { DANGER } else { COPPER };
        let col = cx.tw.c(("layer-c", id), col, 0.7);
        let fw = cx.tw.f(("layer", id), *v as f32, 0.3, Ease::LINEAR);
        w::bar(&p, Rect::from_min_size(pos2(lx, ly), vec2(col_w, 3.0)), fw, col, TRACK);
        let label = if *v >= 1.0 {
            "done".to_string()
        } else if bad {
            "stalled".to_string()
        } else if size.is_empty() {
            format!("{}%", (v * 100.0).round() as i64)
        } else {
            size.clone()
        };
        let short: String = id.chars().take(7).collect();
        w::text(&p, ui, lx, ly + 3.0 + 7.0 + 8.0, &short, mono(11.5), MUTED);
        w::text_right(&p, ui, lx + col_w, ly + 3.0 + 7.0 + 8.0, &label, mono(11.5), MUTED);
    }
}
