//! Linux machines: one card each, with a tiny live shell, the machine's
//! address and local URL. With Patina's engine, the shell opens a real one
//! in your terminal app, and the info button shows its details.

use egui::{pos2, vec2, Align2, CursorIcon, Id, Rect, Sense, Stroke, Ui, UiBuilder};

use super::{enter, Cx};
use crate::icons::{self, Ink};
use crate::sim::{Kind, St};
use crate::theme::*;
use crate::tween::{rise, Ease};
use crate::widgets::{self as w, Btn};

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let ms: Vec<_> = cx.s.ms.iter().filter(|m| !m.gone).cloned().collect();
    let wd = ui.available_width();
    let origin = ui.cursor().min;
    let gap = 20.0;
    let cols = (((wd + gap) / (300.0 + gap)).floor() as usize).max(1);
    let card_w = (wd - gap * (cols - 1) as f32) / cols as f32;
    let card_h = 20.0 + 34.0 + 16.0 + 112.0 + 16.0 + 18.0 + 4.0 + 18.0 + 20.0;
    let named = !cx.s.info.stale;
    let n = ms.len() + 1;
    let rows = n.div_ceil(cols);
    let view_at = cx.s.view_at;
    let now = cx.now;
    let live_mode = cx.s.is_live();

    for i in 0..n {
        let (col, row) = (i % cols, i / cols);
        let rect = Rect::from_min_size(origin + vec2(col as f32 * (card_w + gap), row as f32 * (card_h + gap)), vec2(card_w, card_h));
        if i == ms.len() {
            // "New machine" card
            let (a, dy) = enter(cx.tw, view_at, 0.26, 0.55);
            let r = rect.translate(vec2(0.0, dy));
            let mut cui = ui.new_child(UiBuilder::new().max_rect(r));
            cui.multiply_opacity(a);
            let resp = cui.interact(r, Id::new("add-machine"), Sense::click());
            if resp.hovered() {
                cui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            let hv = cx.tw.f("add-hv", if resp.hovered() { 1.0 } else { 0.0 }, 0.25, Ease::EASE);
            let p = cui.painter().clone();
            w::dashed_rrect(&p, r, 14.0, Stroke::new(1.0_f32, mix(DASH, MUTED, hv)), 4.0, 3.0);
            let col = mix(TEXT2, TEXT, hv);
            let cy = r.center().y;
            icons::paint(&cui, Rect::from_center_size(pos2(r.center().x, cy - 26.0), vec2(20.0, 20.0)), icons::PLUS, Ink::Stroke(1.4), col);
            let g1 = w::g(&cui, "New machine", medium(13.5), col);
            w::put(&p, &g1, pos2(r.center().x, cy + 2.0), Align2::CENTER_CENTER);
            let g2 = w::g(&cui, "Ubuntu, Debian, Fedora, Arch or Alpine", sans(12.5), MUTED);
            w::put(&p, &g2, pos2(r.center().x, cy + 26.0), Align2::CENTER_CENTER);
            if resp.clicked() {
                cx.fx.push(Box::new(|s| s.open_mk()));
            }
            continue;
        }
        let m = &ms[i];
        let delay = if m.added { 0.0 } else { 0.12 + i as f32 * 0.07 };
        let (a, dy) = enter(cx.tw, view_at.max(m.born), delay, 0.55);
        let r = rect.translate(vec2(0.0, dy));
        let mut cui = ui.new_child(UiBuilder::new().max_rect(r));
        cui.multiply_opacity(a);
        let p = cui.painter().clone();
        w::card(&p, r, 14.0, CARD, CARD_EDGE);
        let x = r.left() + 20.0;
        let right = r.right() - 20.0;
        let hy = r.top() + 20.0 + 17.0;
        cx.ambient |= w::status_dot(&p, cx.tw, Id::new(("mdot", &m.id)), pos2(x + 5.0, hy), m.st, false, m.fresh_at);
        let name = w::g_ls(&cui, &m.id, semibold(16.0), TEXT, -0.16);
        let nr = w::put(&p, &name, pos2(x + 22.0, hy - 9.0), Align2::LEFT_CENTER);
        let dr_ = w::text(&p, &cui, x + 22.0, hy + 10.0, &m.distro, sans(12.5), TEXT2);
        let head = cui.interact(nr.union(dr_), Id::new(("mhead", &m.id)), Sense::click());
        if head.hovered() {
            cui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let del = Btn::icon(icons::TRASH, Ink::Stroke(1.3)).color(MUTED);
        let (dr, drr) = del.at_right(&mut cui, right, hy, Id::new(("mdel", &m.id)));
        if dr.clicked() {
            let id = m.id.clone();
            cx.fx.push(Box::new(move |s| s.ask_asset(Kind::Ms, &id)));
        }
        let (ir, irr) = Btn::icon(icons::INFO, Ink::Stroke(1.3)).color(MUTED).at_right(&mut cui, drr.left() - 2.0, hy, Id::new(("minfo", &m.id)));
        if ir.clicked() || head.clicked() {
            let id = m.id.clone();
            cx.fx.push(Box::new(move |s| s.open_machine_info(&id)));
        }
        let drr = irr;
        let going = m.st == St::Running;
        let label = match m.st {
            St::Running => "Stop",
            St::Stopped => "Start",
            St::Starting => "Starting",
            _ => "Stopping",
        };
        let (tr, _) = Btn::ghost(label).h(30.0).pad(13.0).radius(8.0).at_right(&mut cui, drr.left() - 12.0, hy, Id::new(("mtog", &m.id)));
        if tr.clicked() {
            let id = m.id.clone();
            cx.fx.push(Box::new(move |s| s.flip_machine(&id)));
        }

        // shell
        let term = Rect::from_min_size(pos2(x, r.top() + 20.0 + 34.0 + 16.0), vec2(right - x, 112.0));
        w::fill(&p, term, 10.0, TERM);
        if going {
            let t = (now - view_at.max(m.since)) as f32;
            if t < 2.2 {
                cx.tw.busy = true;
            }
            cx.ambient = true;
            let lx = term.left() + 14.0;
            let mut ly = term.top() + 14.0 + 9.6;
            let ps = format!("{}:~$", m.id);
            let pr = w::text(&p, &cui, lx, ly, &ps, mono(12.0), VERDIGRIS);
            let typed = ((((t - 0.5) / 1.0).clamp(0.0, 1.0)) * 9.0).floor() as usize;
            w::text(&p, &cui, pr.right() + 8.0, ly, &"uname -sm"[..typed.min(9)], mono(12.0), TEXT);
            ly += 19.2 + 4.0;
            let t0 = now - t as f64;
            let (oa, ody, _) = rise(now, t0, 1.6, 0.4, 10.0);
            if oa > 0.0 {
                let mut op = p.clone();
                op.multiply_opacity(oa);
                let uname = if m.arch == "amd64" { "Linux x86_64" } else { "Linux aarch64" };
                w::text(&op, &cui, lx, ly + ody, uname, mono(12.0), TEXT);
            }
            ly += 19.2 + 4.0;
            let (ca, cdy, _) = rise(now, t0, 1.8, 0.3, 10.0);
            if ca > 0.0 {
                let mut op = p.clone();
                op.multiply_opacity(ca);
                let pr = w::text(&op, &cui, lx, ly + cdy, &ps, mono(12.0), VERDIGRIS);
                w::cursor_block(&op, Rect::from_min_size(pos2(pr.right() + 8.0, ly + cdy - 7.5), vec2(7.0, 15.0)), now);
            }
            if live_mode {
                let b = Btn::ghost("Open in Terminal").h(30.0).pad(12.0).radius(8.0).font(medium(12.5)).color(TEXT2);
                let (r, _) = b.at_right(&mut cui, term.right() - 12.0, term.bottom() - 12.0 - 15.0, Id::new(("mterm", &m.id)));
                if r.clicked() {
                    let id = m.id.clone();
                    cx.fx.push(Box::new(move |s| s.open_machine_shell(&id)));
                }
            }
        } else if m.st == St::Starting && !m.note.is_empty() {
            // Creating it: the stage and how far along, a bar, and the detail.
            cx.ambient = true;
            let (stage, detail) = m.note.split_once(" · ").unwrap_or((m.note.as_str(), ""));
            let bx = term.left() + 22.0;
            let bw = term.width() - 44.0;
            let ty = term.top() + 34.0;
            w::text(&p, &cui, bx, ty, stage, medium(13.0), TEXT2);
            let bar = Rect::from_min_size(pos2(bx, ty + 16.0), vec2(bw, 6.0));
            match m.progress {
                Some(f) => {
                    let shown = cx.tw.f(("mprog", &m.id), f, 0.6, Ease::OUT);
                    w::bar(&p, bar, shown, COPPER, COPPER.gamma_multiply(0.18));
                    w::text_right(&p, &cui, bar.right(), ty, &format!("{:.0}%", shown * 100.0), mono(12.0), TEXT2);
                }
                None => {
                    w::fill(&p, bar, 3.0, COPPER.gamma_multiply(0.18));
                    let ph = ((now * 0.8) % 1.0) as f32;
                    let seg_w = bw * 0.3;
                    let sx = bar.left() + (bw + seg_w) * ph - seg_w;
                    let seg = Rect::from_min_max(pos2(sx.max(bar.left()), bar.top()), pos2((sx + seg_w).min(bar.right()), bar.bottom()));
                    if seg.width() > 0.0 {
                        w::fill(&p, seg, 3.0, COPPER);
                    }
                }
            }
            if !detail.is_empty() {
                w::text(&p, &cui, bx, bar.bottom() + 18.0, detail, sans(12.5), MUTED);
            }
        } else {
            let note = match m.st {
                St::Starting => format!("Booting {}", m.id),
                St::Stopping => "Shutting down".into(),
                _ => format!("Start {} to open a shell.", m.id),
            };
            let g = w::g(&cui, &note, sans(13.0), MUTED);
            w::put(&p, &g, term.center(), Align2::CENTER_CENTER);
        }
        let meta = format!(
            "{} · {}",
            m.arch,
            if going && !m.up.is_empty() {
                m.up.clone()
            } else if going {
                "running".into()
            } else {
                match m.st {
                    St::Starting => "starting".into(),
                    St::Stopping => "stopping".into(),
                    _ => "stopped".into(),
                }
            }
        );
        w::text(&p, &cui, x, term.bottom() + 16.0 + 9.0, &meta, sans(12.5), MUTED);

        // Where to reach it from the Mac.
        let ly = term.bottom() + 16.0 + 18.0 + 4.0 + 9.0;
        if m.ip.is_empty() {
            w::text(&p, &cui, x, ly, "Gets an address when it starts", sans(12.5), MUTED);
        } else {
            let host = if named || !live_mode { crate::engine::machines::domain(&m.id) } else { m.ip.clone() };
            let link = Rect::from_min_size(pos2(x, ly - 9.0), vec2(w::text_w(&cui, &host, mono(12.0)), 18.0));
            let lr = cui.interact(link, Id::new(("murl", &m.id)), Sense::click());
            let can = going;
            if can && lr.hovered() {
                cui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            let hv = cx.tw.f(("murl-hv", &m.id), if can && lr.hovered() { 1.0 } else { 0.0 }, 0.2, Ease::EASE);
            let col = if can { mix(VERDIGRIS, TEXT, hv) } else { MUTED };
            let r = w::text(&p, &cui, x, ly, &host, mono(12.0), col);
            if can && hv > 0.01 {
                p.line_segment([pos2(r.left(), r.bottom() + 0.5), pos2(r.right(), r.bottom() + 0.5)], Stroke::new(1.0_f32, col.gamma_multiply(hv)));
            }
            if host != m.ip {
                w::text(&p, &cui, r.right() + 8.0, ly, &format!("· {}", m.ip), sans(12.5), MUTED);
            }
            if can && lr.clicked() {
                let url = format!("http://{host}");
                cx.fx.push(Box::new(move |s| s.open_url(url)));
            }
        }
    }
    let h = rows as f32 * card_h + (rows - 1) as f32 * gap;
    ui.advance_cursor_after_rect(Rect::from_min_size(origin, vec2(wd, h)));
}
