//! Containers: grouped list with live CPU sparklines, the row menu, and the
//! detail panel with logs, a terminal and details.

use std::f32::consts::PI;

use egui::{pos2, vec2, Align2, Area, Context, CursorIcon, Id, Order, Pos2, Rect, Sense, Stroke, Ui, UiBuilder};

use super::{enter, swap, Cx};
use crate::icons::{self, Ink};
use crate::sim::{Container, Engine, Filter, Job, ModalKind, St, State, Tab};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

pub fn up(c: &Container) -> bool {
    matches!(c.st, St::Running | St::Starting | St::Paused)
}

pub fn matches(c: &Container, f: Filter, q: &str) -> bool {
    (f == Filter::All || (f == Filter::Running) == up(c)) && (q.is_empty() || c.id.contains(q) || c.image.contains(q))
}

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let wd = ui.available_width();
    let aside_w = 348.0;
    let gap = 24.0;
    let stacked = wd < 460.0 + gap + aside_w;
    let list_w = if stacked { wd } else { wd - gap - aside_w };
    let origin = ui.cursor().min;
    let list_h = list(ui, cx, origin, list_w);
    let aside_origin = if stacked { origin + vec2(0.0, list_h + 24.0) } else { origin + vec2(list_w + gap, 0.0) };
    let aside_h = detail(ui, cx, aside_origin, if stacked { wd } else { aside_w });
    let total = if stacked { list_h + 24.0 + aside_h } else { list_h.max(aside_h) };
    ui.advance_cursor_after_rect(Rect::from_min_size(origin, vec2(wd, total)));
}

// -------------------------------------------------------------------- list

fn list(ui: &mut Ui, cx: &mut Cx, origin: Pos2, wd: f32) -> f32 {
    let cs: Vec<Container> = cx.s.cs.clone();
    let filter = cx.s.filter;
    let q = cx.s.q.trim().to_lowercase();
    let sel = cx.s.sel.clone();
    let menu = cx.s.menu.clone();
    let live = cx.s.live();
    let booting = cx.s.engine == Engine::Booting;
    let view_at = cx.s.view_at;
    let list_o = cx.tw.f("list-o", if booting { 0.45 } else { 1.0 }, 0.5, Ease::EASE);
    let mut y = origin.y;
    let mut n = 0usize;
    let mut menu_row: Option<(Container, Rect)> = None;

    for (gid, glabel) in [("storefront", "storefront"), ("standalone", "Standalone")] {
        let grp: Vec<&Container> = cs.iter().filter(|c| c.grp == gid).collect();
        let mine: Vec<&Container> = grp.iter().copied().filter(|c| !c.gone).collect();
        let open = mine.iter().any(|c| matches(c, filter, &q));
        let going = mine.iter().filter(|c| c.st == St::Running).count();
        let hh = cx.tw.f(("gh", gid), if open { 37.0 } else { 0.0 }, 0.4, Ease::OUT);
        let ho = cx.tw.f(("go", gid), if open { 1.0 } else { 0.0 }, 0.3, Ease::EASE);
        let delay = 0.2 + n as f32 * 0.04;
        n += 1;
        if hh > 0.5 {
            let clip = Rect::from_min_size(pos2(origin.x, y), vec2(wd, hh));
            let (a, dy) = enter(cx.tw, view_at, delay, 0.55);
            let mut gui = ui.new_child(UiBuilder::new().max_rect(clip));
            gui.set_clip_rect(clip.intersect(ui.clip_rect()));
            gui.multiply_opacity(a * ho * list_o);
            let r = Rect::from_min_size(pos2(origin.x, y + dy), vec2(wd, 37.0));
            let p = gui.painter().clone();
            let cy = r.top() + 6.0 + 15.5;
            let lab = w::g_ls(&gui, glabel, semibold(12.0), TEXT2, 0.48);
            let lr = w::put(&p, &lab, pos2(r.left() + 14.0, cy), Align2::LEFT_CENTER);
            let meta = format!("{}{} of {} running", if gid == "storefront" { "Compose · " } else { "" }, going, mine.len());
            w::text(&p, &gui, lr.right() + 10.0, cy, &meta, sans(12.0), MUTED);
            if gid == "storefront" && live {
                let label = if going > 0 { "Stop all" } else { "Start all" };
                let b = Btn::plain(label).h(24.0).pad(9.0).radius(6.0).font(sans(12.0)).color(MUTED);
                let (resp, _) = b.at_right(&mut gui, r.right() - 8.0, cy, Id::new(("grp-all", gid)));
                if resp.clicked() {
                    let stop = going > 0;
                    cx.fx.push(Box::new(move |s| s.group_all("storefront", stop)));
                }
            }
        }
        y += hh;

        for c in grp {
            let vis = !c.gone && matches(c, filter, &q);
            let rh = cx.tw.f(("rh", &c.id), if vis { 54.0 } else { 0.0 }, 0.4, Ease::OUT);
            let ro = cx.tw.f(("ro", &c.id), if vis { 1.0 } else { 0.0 }, 0.3, Ease::EASE);
            let delay = if c.added { 0.0 } else { 0.2 + n as f32 * 0.04 };
            n += 1;
            if rh > 0.5 {
                let clip = Rect::from_min_size(pos2(origin.x, y), vec2(wd, rh));
                let (a, dy) = enter(cx.tw, view_at.max(c.born), delay, 0.55);
                let mut rui = ui.new_child(UiBuilder::new().max_rect(clip));
                rui.set_clip_rect(clip.intersect(ui.clip_rect()));
                rui.multiply_opacity(a * ro * list_o);
                let row = Rect::from_min_size(pos2(origin.x, y + 1.0 + dy), vec2(wd, 52.0));
                container_row(&mut rui, cx, c, row, c.id == sel, vis);
                if vis && menu.as_deref() == Some(c.id.as_str()) {
                    menu_row = Some((c.clone(), row));
                }
            }
            y += rh;
        }
    }
    let list_h = y - origin.y;
    if let Some((c, row)) = menu_row {
        context_menu(ui.ctx(), cx, &c, row, origin, wd, list_h);
    }
    list_h
}

fn container_row(ui: &mut Ui, cx: &mut Cx, c: &Container, row: Rect, selected: bool, interactive: bool) {
    let p = ui.painter().clone();
    let hovered = interactive && ui.rect_contains_pointer(row);
    let hv = cx.tw.f(("row-hv", &c.id), if hovered { 1.0 } else { 0.0 }, 0.2, Ease::EASE);
    if selected {
        w::card(&p, row, 10.0, ROW_SEL, ROW_SEL_EDGE);
    } else if hv > 0.01 {
        w::fill(&p, row, 10.0, ROW_HOVER.gamma_multiply(hv));
    }
    let cy = row.center().y;
    let more_r = Rect::from_min_size(pos2(row.right() - 8.0 - 30.0, cy - 15.0), vec2(30.0, 30.0));
    let tog_r = more_r.translate(vec2(-32.0, 0.0));
    let pick_r = Rect::from_min_max(row.min, pos2(tog_r.left() - 2.0, row.bottom()));

    let id = c.id.clone();
    let pick = ui.interact(pick_r, Id::new(("pick", &id)), Sense::click());
    if pick.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if pick.clicked() && interactive {
        let id = id.clone();
        cx.fx.push(Box::new(move |s| s.pick(&id)));
    }

    // Columns: 22px dot | name (flex) | port 62 | bars 46 | cpu 50 | mem 62
    let x = pick_r.left() + 14.0;
    let name_x = x + 22.0 + 14.0;
    let fixed = 14.0 + 62.0 + 14.0 + 46.0 + 14.0 + 50.0 + 14.0 + 62.0;
    let name_w = (pick_r.right() - 6.0 - fixed - name_x).max(40.0);
    let port_x = name_x + name_w + 14.0;
    let bars_x = port_x + 62.0 + 14.0;
    let cpu_r = bars_x + 46.0 + 14.0 + 50.0;
    let mem_r = cpu_r + 14.0 + 62.0;

    cx.ambient |= w::status_dot(&p, cx.tw, Id::new(("dot", &c.id)), pos2(x + 5.0, cy), c.st, c.bad, c.fresh_at);

    let run = c.st == St::Running;
    let warm = c.st == St::Starting;
    let busy = warm || c.st == St::Stopping;
    let err = c.st == St::Error;
    let hold = c.st == St::Paused;
    let stoppable = run || hold || c.loops > 0;

    let tag = if c.loops > 0 {
        format!("restarting ×{}", c.loops)
    } else if hold {
        "paused".to_string()
    } else {
        c.tag.clone()
    };
    let (tag_c, tag_e) = if err {
        (DANGER_TEXT, ERR_EDGE)
    } else if hold {
        (TEXT2, BTN_EDGE)
    } else {
        (COPPER_TEXT, WARN_EDGE)
    };
    let name_c = cx.tw.c(("name-c", &c.id), if c.st == St::Stopped { TEXT2 } else { TEXT }, 0.4);
    let tag_g = (!tag.is_empty()).then(|| w::g(ui, &tag, sans(11.0), tag_c));
    let tag_w = tag_g.as_ref().map_or(0.0, |g| g.size().x + 14.0);
    let name_max = name_w - if tag_w > 0.0 { tag_w + 8.0 } else { 0.0 };
    let line1 = cy - 8.0;
    let line2 = cy + 9.8;
    let name_g = w::g_elide(ui, &c.id, medium(14.0), name_c, name_max);
    let nr = w::put(&p, &name_g, pos2(name_x, line1), Align2::LEFT_CENTER);
    if let Some(tg) = tag_g {
        let tr = Rect::from_min_size(pos2(nr.right() + 8.0, line1 - 8.5), vec2(tag_w, 17.0));
        w::edge(&p, tr, 6.0, tag_e);
        w::put(&p, &tg, tr.center(), Align2::CENTER_CENTER);
    }
    let img_g = w::g_elide(ui, &c.image, mono(11.5), MUTED, name_w);
    w::put(&p, &img_g, pos2(name_x, line2), Align2::LEFT_CENTER);

    if !c.port.is_empty() {
        w::text(&p, ui, port_x, cy, &format!(":{}", c.port), mono(12.0), TEXT2);
    }

    let bar_c = if run {
        if c.bad {
            COPPER
        } else {
            VERDIGRIS
        }
    } else if hold {
        BAR_PAUSED
    } else if warm {
        BAR_WARM
    } else {
        BAR_IDLE
    };
    let bar_c = cx.tw.c(("bar-c", &c.id), bar_c, 0.6);
    for i in 0..12 {
        let v = cx.tw.f(("bar", &c.id, i), c.bars[i], 1.0, Ease::STD);
        let h = 18.0 * v;
        let bx = bars_x + i as f32 * 4.0;
        p.rect_filled(Rect::from_min_max(pos2(bx, cy + 9.0 - h), pos2(bx + 2.0, cy + 9.0)), 1.0, bar_c);
    }

    let cpu = if run { format!("{:.1}%", c.cpu) } else { "—".into() };
    let mem = if run || hold { format!("{} MB", c.mem as i64) } else { "—".into() };
    w::text_right(&p, ui, cpu_r, cy, &cpu, mono(12.0), TEXT);
    w::text_right(&p, ui, mem_r, cy, &mem, mono(12.0), TEXT2);

    // Start / stop
    let tog = if busy && !stoppable {
        let resp = Btn::plain("").w(30.0).at(ui, tog_r, Id::new(("tog", &id)));
        let rot = ((cx.now % 0.8) / 0.8) as f32 * std::f32::consts::TAU;
        w::arc(&p, tog_r.center(), 5.5, rot, rot + 1.5 * PI, Stroke::new(1.6_f32, TEXT2));
        cx.ambient = true;
        resp
    } else {
        let ico = if stoppable { icons::STOP } else { icons::PLAY };
        Btn::icon(ico, Ink::Fill).at(ui, tog_r, Id::new(("tog", &id)))
    };
    if tog.clicked() && interactive {
        let id = id.clone();
        cx.fx.push(Box::new(move |s| s.toggle(&id)));
    }
    let more = Btn::icon(icons::MORE, Ink::Fill).at(ui, more_r, Id::new(("more", &id)));
    if more.clicked() && interactive {
        cx.fx.push(Box::new(move |s| s.toggle_menu(&id)));
    }
}

// -------------------------------------------------------------------- menu

struct Item {
    label: String,
    ok: bool,
    danger: bool,
    go: Option<Job>,
}

fn item(label: impl Into<String>, ok: bool, go: impl FnOnce(&mut State) + 'static) -> Item {
    Item { label: label.into(), ok, danger: false, go: Some(Box::new(go)) }
}

fn context_menu(ctx: &Context, cx: &mut Cx, c: &Container, row: Rect, origin: Pos2, list_w: f32, list_h: f32) {
    let run = c.st == St::Running;
    let paused = c.st == St::Paused;
    let (id, port, hex) = (c.id.clone(), c.port.clone(), c.hex.clone());
    let sep = || Item { label: String::new(), ok: true, danger: false, go: None };
    let items = vec![
        item("Open in browser", run && !port.is_empty(), {
            let port = port.clone();
            move |s| s.toast_say(format!("Opening localhost:{port}"), "", None)
        }),
        item("Open a terminal", run, {
            let id = id.clone();
            move |s| {
                s.pick(&id);
                s.set_tab(Tab::Term);
            }
        }),
        item("View logs", true, {
            let id = id.clone();
            move |s| {
                s.pick(&id);
                s.set_tab(Tab::Logs);
            }
        }),
        item("Copy ID", true, {
            let hex = hex.clone();
            move |s| s.toast_say(format!("Copied {hex}"), "", None)
        }),
        sep(),
        item(if paused { "Resume" } else { "Pause" }, run || paused, {
            let id = id.clone();
            move |s| s.pause_box(&id)
        }),
        item("Restart", run, {
            let id = id.clone();
            move |s| s.restart(&id)
        }),
        sep(),
        Item {
            label: "Delete".into(),
            ok: true,
            danger: true,
            go: Some(Box::new({
                let id = id.clone();
                move |s: &mut State| s.ask_modal(ModalKind::Delete, &id)
            })),
        },
    ];
    let mh = 12.0 + 7.0 * 32.0 + 2.0 * 11.0;
    let row_top = row.top() - 1.0;
    let down = (row_top - origin.y) + 46.0 + mh <= list_h + 30.0;
    let top = if down { row_top + 46.0 } else { (row_top - mh + 10.0).max(origin.y) };
    let left = origin.x + list_w - 8.0 - 216.0;
    let screen = ctx.screen_rect();

    // Click anywhere else to close.
    Area::new(Id::new("menu-block")).order(Order::Middle).fixed_pos(screen.min).constrain(false).show(ctx, |ui| {
        let r = ui.allocate_rect(screen, Sense::click());
        if r.clicked() {
            cx.fx.push(Box::new(|s| s.menu = None));
        }
    });

    let t = ((cx.now - cx.s.menu_at) / 0.18) as f32;
    let e = Ease::POP.at(t.clamp(0.0, 1.0));
    if t < 1.0 {
        cx.tw.busy = true;
    }
    let scale = 0.96 + 0.04 * e;
    let full = Rect::from_min_size(pos2(left, top), vec2(216.0, mh));
    // scale from the top-right corner
    let rect = Rect::from_min_size(
        pos2(full.right() - 216.0 * scale, full.top() + 14.0 * (1.0 - e)),
        vec2(216.0 * scale, mh * scale),
    );
    Area::new(Id::new("ctx-menu")).order(Order::Foreground).fixed_pos(full.min).constrain(false).show(ctx, |ui| {
        ui.multiply_opacity(e.clamp(0.0, 1.0));
        let p = ui.painter().clone();
        w::shadow(&p, rect, 12.0, 44.0, 18.0, rgba(0, 0, 0, 0.5));
        w::card(&p, rect, 12.0, MENU, BTN_EDGE);
        let mut y = rect.top() + 6.0 * scale;
        for (i, it) in items.into_iter().enumerate() {
            let Some(go) = it.go else {
                p.line_segment(
                    [pos2(rect.left() + 10.0, y + 5.5 * scale), pos2(rect.right() - 10.0, y + 5.5 * scale)],
                    Stroke::new(1.0_f32, BTN_EDGE),
                );
                y += 11.0 * scale;
                continue;
            };
            let ir = Rect::from_min_size(pos2(rect.left() + 6.0, y), vec2(rect.width() - 12.0, 32.0 * scale));
            let resp = ui.interact(ir, Id::new(("ctx-item", i)), Sense::click());
            if resp.hovered() {
                w::fill(&p, ir, 7.0, MENU_HOVER);
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            let col = if it.danger { DANGER_TEXT } else { TEXT };
            let col = if it.ok { col } else { col.gamma_multiply(0.4) };
            w::text(&p, ui, ir.left() + 10.0, ir.center().y, &it.label, sans(13.5), col);
            if resp.clicked() {
                let ok = it.ok;
                let copy = (it.label == "Copy ID").then(|| hex.clone());
                cx.fx.push(Box::new(move |s| {
                    s.menu = None;
                    if ok {
                        go(s);
                    }
                }));
                if let Some(hex) = copy {
                    ui.ctx().copy_text(hex);
                }
            }
            y += 32.0 * scale;
        }
        ui.advance_cursor_after_rect(full);
    });
}

// ------------------------------------------------------------------ detail

fn detail(ui: &mut Ui, cx: &mut Cx, origin: Pos2, wd: f32) -> f32 {
    let s = &*cx.s;
    let Some(c0) = s.alive().find(|c| c.id == s.sel).or_else(|| s.alive().next()).cloned() else {
        return 0.0;
    };
    let logs = s.logs.get(&c0.id).cloned().unwrap_or_default();
    let (tab, panel_at, tab_at, log_at, view_at) = (s.tab, s.panel_at, s.tab_at, s.log_at, s.view_at);
    let now = cx.now;
    let iw = wd - 40.0;
    let run = c0.st == St::Running;
    let held = c0.st == St::Paused;

    let state = if run {
        if c0.bad {
            "Running · unhealthy".to_string()
        } else {
            format!("Running · up {}", c0.up)
        }
    } else {
        match c0.st {
            St::Starting if c0.loops > 0 => format!("Restarting · attempt {}", c0.loops),
            St::Starting => "Starting".into(),
            St::Stopping => "Stopping".into(),
            St::Paused => "Paused".into(),
            St::Error => format!("Failed · {}", if c0.tag.is_empty() { "exited" } else { &c0.tag }),
            _ => "Stopped · exited (0)".into(),
        }
    };
    let st_c = if run {
        if c0.bad {
            COPPER
        } else {
            VERDIGRIS
        }
    } else {
        match c0.st {
            St::Starting => COPPER,
            St::Error => DANGER,
            St::Paused => VERDIGRIS,
            _ => DIM,
        }
    };
    let name_g = w::g_ls(ui, &c0.id, semibold(19.0), TEXT, -0.285);
    let why_g = (!c0.why.is_empty()).then(|| w::g_wrap(ui, &c0.why, sans(12.5), TEXT, iw - 28.0 - 17.0, 18.75));
    let head_h = 48.0;
    let why_h = why_g.as_ref().map_or(0.0, |g| g.size().y + 22.0 + 18.0);
    let stats_h = 62.0;
    let total = 20.0 + head_h + 18.0 + why_h + stats_h + 18.0 + 35.0 + 18.0 + 236.0 + 20.0;

    let (a, dy) = enter(cx.tw, view_at, 0.3, 0.6);
    let card = Rect::from_min_size(origin + vec2(0.0, dy), vec2(wd, total));
    let mut aui = ui.new_child(UiBuilder::new().max_rect(card));
    aui.multiply_opacity(a);
    let p = aui.painter().clone();
    w::card(&p, card, 14.0, CARD, CARD_EDGE);
    let x = card.left() + 20.0;
    let right = card.right() - 20.0;

    // ---- the part that swaps when the selection changes
    let (pa, pdy) = swap(cx.tw, panel_at, 0.4);
    let mut tui = aui.new_child(UiBuilder::new().max_rect(card));
    tui.multiply_opacity(pa);
    let tp = tui.painter().clone();
    let mut y = card.top() + 20.0 + pdy;
    w::put(&tp, &name_g, pos2(x, y + 13.0), Align2::LEFT_CENTER);
    let stc = cx.tw.c("detail-st", st_c, 0.7);
    let sy = y + 26.6 + 4.0 + 8.75;
    w::small_dot(&tp, pos2(x + 3.5, sy), stc);
    w::text(&tp, &tui, x + 15.0, sy, &state, sans(12.5), TEXT2);
    let btn = Btn::ghost(if run || held || c0.loops > 0 { "Stop" } else { "Start" }).h(30.0).pad(13.0).radius(8.0);
    let (tr, tr_rect) = btn.at_right(&mut tui, right, y + 15.0, Id::new("detail-toggle"));
    if tr.clicked() {
        let id = c0.id.clone();
        cx.fx.push(Box::new(move |s| s.toggle(&id)));
    }
    let restart = Btn::icon(icons::RESTART, Ink::Stroke(1.4));
    let (rr, _) = restart.at_right(&mut tui, tr_rect.left() - 8.0, y + 15.0, Id::new("detail-restart"));
    if rr.clicked() {
        let id = c0.id.clone();
        cx.fx.push(Box::new(move |s| s.restart(&id)));
    }
    y += head_h + 18.0;

    if let Some(g) = &why_g {
        let err = c0.st == St::Error;
        let r = Rect::from_min_size(pos2(x, y), vec2(iw, g.size().y + 22.0));
        w::card(&tp, r, 10.0, if err { ERR_BG } else { WARN_BG }, if err { ERR_EDGE } else { WARN_EDGE });
        w::small_dot(&tp, pos2(x + 14.0 + 3.5, y + 11.0 + 9.5), if err { DANGER } else { COPPER });
        tp.galley(pos2(x + 14.0 + 7.0 + 10.0, y + 11.0), g.clone(), TEXT);
        y += g.size().y + 22.0 + 18.0;
    }

    let col_w = (iw - 16.0) / 2.0;
    let cpu = if run { format!("{:.1}%", c0.cpu) } else { "—".into() };
    let mem = if run || held { format!("{} MB", c0.mem as i64) } else { "—".into() };
    let cpu_f = if run { (c0.cpu * 9.0 / 100.0).min(1.0) as f32 } else { 0.0 };
    let mem_f = if run || held { (c0.mem / 256.0).min(1.0) as f32 } else { 0.0 };
    for (i, (label, val, f)) in [("CPU", cpu, cpu_f), ("Memory", mem, mem_f)].into_iter().enumerate() {
        let cx0 = x + i as f32 * (col_w + 16.0);
        w::text(&tp, &tui, cx0, y + 8.5, label, sans(12.0), MUTED);
        let vg = w::g_ls(&tui, &val, mono(22.0), TEXT, -0.66);
        w::put(&tp, &vg, pos2(cx0, y + 17.0 + 6.0 + 15.0), Align2::LEFT_CENTER);
        let fv = cx.tw.f(("detail-bar", i), f, 1.0, Ease::STD);
        w::bar(&tp, Rect::from_min_size(pos2(cx0, y + 59.0), vec2(col_w, 3.0)), fv, VERDIGRIS, TRACK);
    }

    // ---- tabs (not part of the swap)
    let ty = card.top() + 20.0 + head_h + 18.0 + why_h + stats_h + 18.0;
    let tab_w = iw / 3.0;
    let tabs = [("Logs", Tab::Logs), ("Terminal", Tab::Term), ("Details", Tab::Info)];
    for (i, (label, t)) in tabs.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x + i as f32 * tab_w, ty), vec2(tab_w, 34.0));
        let resp = aui.interact(r, Id::new(("tab", i)), Sense::click());
        if resp.hovered() {
            aui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.clicked() {
            cx.fx.push(Box::new(move |s| s.set_tab(t)));
        }
        let col = cx.tw.c(("tab-c", i), if tab == t { TEXT } else { TEXT2 }, 0.25);
        let g = w::g(&aui, label, medium(13.5), col);
        w::put(&p, &g, r.center(), Align2::CENTER_CENTER);
    }
    p.line_segment([pos2(x, ty + 34.5), pos2(right, ty + 34.5)], Stroke::new(1.0_f32, CARD_EDGE));
    let ti = tabs.iter().position(|(_, t)| *t == tab).unwrap_or(0) as f32;
    let ux = cx.tw.f("tab-ul", ti * tab_w, 0.38, Ease::SPRING_SOFT);
    w::fill(&p, Rect::from_min_size(pos2(x + ux, ty + 33.0), vec2(tab_w, 2.0)), 2.0, TEXT);

    let body = Rect::from_min_size(pos2(x, ty + 35.0 + 18.0), vec2(iw, 236.0));
    let (ta, tdy) = swap(cx.tw, tab_at, 0.35);
    let mut bui = aui.new_child(UiBuilder::new().max_rect(body));
    bui.set_clip_rect(body.intersect(aui.clip_rect()));
    bui.multiply_opacity(ta);
    let bp = bui.painter().clone();
    let body = body.translate(vec2(0.0, tdy));

    match tab {
        Tab::Logs => {
            let lt = ((now - log_at) / 0.4) as f32;
            let shift = if lt < 1.0 {
                cx.tw.busy = true;
                22.0 * (1.0 - Ease::OUT.at(lt.max(0.0)))
            } else {
                0.0
            };
            let n = logs.len();
            let total_h = n as f32 * 22.0 - 5.0;
            for (i, l) in logs.iter().enumerate() {
                let top = body.bottom() - total_h + i as f32 * 22.0 + shift;
                let cy = top + 8.5;
                let mut lp = bp.clone();
                if i + 1 == n && lt < 1.0 {
                    let e = Ease::EASE.at(lt.max(0.0));
                    lp.multiply_opacity(e);
                }
                let tg = w::g(&bui, &l.t, mono(11.5), MUTED);
                let tr = w::put(&lp, &tg, pos2(body.left(), cy), Align2::LEFT_CENTER);
                let col = match l.k {
                    1 => VERDIGRIS,
                    2 => COPPER,
                    3 => DANGER_TEXT,
                    _ => CODE,
                };
                let mg = w::g_elide(&bui, &l.m, mono(11.5), col, body.width() - tr.width() - 10.0);
                w::put(&lp, &mg, pos2(tr.right() + 10.0, cy), Align2::LEFT_CENTER);
            }
        }
        Tab::Term if run => {
            w::fill(&bp, body, 10.0, TERM);
            let t = (now - tab_at.max(panel_at).max(c0.since).max(view_at)) as f32;
            if t < 2.0 {
                cx.tw.busy = true;
            }
            cx.ambient = true;
            let lx = body.left() + 14.0;
            let lh = 19.2 + 6.0;
            let mut ly = body.top() + 14.0 + 9.6;
            w::text(&bp, &bui, lx, ly, &format!("sh in {}", c0.id), mono(12.0), MUTED);
            ly += lh;
            let pr = w::text(&bp, &bui, lx, ly, "/ #", mono(12.0), VERDIGRIS);
            let typed = ((((t - 0.3) / 0.9).clamp(0.0, 1.0)) * 8.0).floor() as usize;
            w::text(&bp, &bui, pr.right() + 8.0, ly, &"hostname"[..typed.min(8)], mono(12.0), TEXT);
            ly += lh;
            let (oa, ody, _) = crate::tween::rise(now, now - t as f64, 1.3, 0.4, 10.0);
            if oa > 0.0 {
                let mut op = bp.clone();
                op.multiply_opacity(oa);
                w::text(&op, &bui, lx, ly + ody, &c0.hex, mono(12.0), TEXT);
            }
            ly += lh;
            let (ca, cdy, _) = crate::tween::rise(now, now - t as f64, 1.5, 0.3, 10.0);
            if ca > 0.0 {
                let mut op = bp.clone();
                op.multiply_opacity(ca);
                let pr = w::text(&op, &bui, lx, ly + cdy, "/ #", mono(12.0), VERDIGRIS);
                w::cursor_block(&op, Rect::from_min_size(pos2(pr.right() + 8.0, ly + cdy - 7.5), vec2(7.0, 15.0)), now);
            }
        }
        Tab::Term => {
            w::fill(&bp, body, 10.0, TERM);
            let g = w::g(&bui, &format!("Start {} to open a shell.", c0.id), sans(13.5), MUTED);
            w::put(&bp, &g, body.center(), Align2::CENTER_CENTER);
        }
        Tab::Info => {
            let rows = [
                ("Image", c0.image.clone()),
                ("Ports", if c0.port.is_empty() { "none published".into() } else { format!("{} → {}/tcp", c0.port, c0.port) }),
                ("Network", if c0.grp == "storefront" { "storefront_default".into() } else { "bridge".into() }),
                ("Project", if c0.grp == "storefront" { "storefront (Compose)".into() } else { "none".into() }),
                ("ID", c0.hex.clone()),
                ("Restart", "unless-stopped".into()),
            ];
            let mut ly = body.top() + 8.75;
            for (k, v) in rows {
                w::text(&bp, &bui, body.left(), ly, k, sans(12.5), MUTED);
                let g = w::g_elide(&bui, &v, mono(12.0), TEXT, body.width() - 84.0);
                w::put(&bp, &g, pos2(body.left() + 84.0, ly), Align2::LEFT_CENTER);
                ly += 17.5 + 13.0;
            }
        }
    }
    total
}
