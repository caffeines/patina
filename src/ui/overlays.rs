//! Dialogs (confirm, create, run, new machine), the command palette and toasts.

use egui::{pos2, vec2, Align2, Area, Context, CursorIcon, Id, Key, Modifiers, Order, Rect, Sense, Stroke, Ui, UiBuilder};

use super::{field, maybe_focus, Cx};
use crate::icons::{self, Ink};
use crate::sim::{Ov, DISTROS, RUN_PICKS};
use crate::theme::*;
use crate::tween::Ease;
use crate::widgets::{self as w, Btn};

pub fn show(ctx: &Context, cx: &mut Cx) {
    if let Some(ov) = cx.s.ov {
        let t = (cx.now - cx.s.ov_at) as f32;
        if t < 0.3 {
            cx.tw.busy = true;
        }
        let td = Ease::EASE.at((t / 0.2).min(1.0));
        let tp = Ease::POP.at((t / 0.28).min(1.0));
        let (dim, pa, pdy) = if cx.s.ov_out { (1.0 - td, (1.0 - tp).max(0.0), 8.0 * tp) } else { (td, tp.min(1.0), 14.0 * (1.0 - tp)) };
        let screen = ctx.screen_rect();

        // Keyboard: Escape closes, Enter submits.
        let (esc, enter) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Escape), i.consume_key(Modifiers::NONE, Key::Enter)));
        if esc {
            cx.fx.push(Box::new(|s| s.close_ov()));
        }

        Area::new(Id::new("overlay")).order(Order::Foreground).fixed_pos(screen.min).constrain(false).show(ctx, |ui| {
            let bg = ui.allocate_rect(screen, Sense::click());
            ui.painter().rect_filled(screen, 0.0, rgba(8, 7, 6, 0.62 * dim));
            match ov {
                Ov::Modal => modal(ui, cx, screen, pa, pdy, enter),
                Ov::Run => run(ui, cx, screen, pa, pdy, enter),
                Ov::Machine => machine(ui, cx, screen, pa, pdy, enter),
                Ov::Palette => {
                    if bg.clicked() {
                        cx.fx.push(Box::new(|s| s.close_ov()));
                    }
                    palette(ui, cx, screen, pa, pdy, enter);
                }
            }
        });
    }
    toast(ctx, cx);
}

/// Card, shadow, and a child ui faded in with the dialog.
fn dialog(ui: &mut Ui, rect: Rect, pa: f32) -> Ui {
    let mut dui = ui.new_child(UiBuilder::new().max_rect(rect));
    dui.multiply_opacity(pa);
    dui.interact(rect, Id::new("dialog-body"), Sense::click());
    let p = dui.painter().clone();
    w::shadow(&p, rect, 16.0, 60.0, 24.0, rgba(0, 0, 0, 0.5));
    w::card(&p, rect, 16.0, DIALOG, BTN_EDGE_SOFT);
    dui
}

fn title_block(ui: &Ui, title: &str, body: &str, iw: f32) -> (std::sync::Arc<egui::Galley>, std::sync::Arc<egui::Galley>) {
    let t = {
        let mut j = w::job(title, semibold(17.0), TEXT, -0.17);
        j.wrap.max_width = iw;
        j.wrap.break_anywhere = true;
        ui.fonts(|f| f.layout_job(j))
    };
    let b = w::g_wrap(ui, body, sans(13.5), TEXT2, iw, 19.0);
    (t, b)
}

fn buttons(ui: &mut Ui, cx: &mut Cx, right: f32, cy: f32, no: &str, yes: Btn) -> (bool, bool) {
    let (yr, yrect) = yes.at_right(ui, right, cy, Id::new("dlg-yes"));
    let (nr, _) = Btn::ghost(no).at_right(ui, yrect.left() - 10.0, cy, Id::new("dlg-no"));
    if nr.clicked() {
        cx.fx.push(Box::new(|s| s.close_ov()));
    }
    (yr.clicked(), nr.clicked())
}

fn modal(ui: &mut Ui, cx: &mut Cx, screen: Rect, pa: f32, pdy: f32, enter: bool) {
    let spec = cx.s.modal_spec();
    let wd = 420.0f32.min(screen.width() - 48.0);
    let iw = wd - 48.0;
    let (tg, bg) = title_block(ui, &spec.title, &spec.body, iw);
    let field_h = if spec.field { 17.0 + 6.0 + 36.0 + 6.0 + 18.0 + 18.0 } else { 0.0 };
    let h = 24.0 + tg.size().y + 8.0 + bg.size().y + 18.0 + field_h + 34.0 + 24.0;
    let rect = Rect::from_center_size(screen.center() + vec2(0.0, pdy), vec2(wd, h));
    let mut dui = dialog(ui, rect, pa);
    let p = dui.painter().clone();
    let x = rect.left() + 24.0;
    let mut y = rect.top() + 24.0;
    p.galley(pos2(x, y), tg.clone(), TEXT);
    y += tg.size().y + 8.0;
    p.galley(pos2(x, y), bg.clone(), TEXT2);
    y += bg.size().y + 18.0;
    if spec.field {
        w::text(&p, &dui, x, y + 8.5, spec.label, sans(12.0), TEXT2);
        y += 17.0 + 6.0;
        let err = !cx.s.modal.err.is_empty();
        let fid = Id::new("modal-field");
        let resp = field(&mut dui, cx.tw, Rect::from_min_size(pos2(x, y), vec2(iw, 36.0)), fid, &mut cx.s.modal.value, spec.ph, err, mono(12.5));
        maybe_focus(cx.s, "modal-field", &resp);
        if resp.changed() {
            cx.s.modal.err.clear();
        }
        y += 36.0 + 6.0;
        if err {
            w::text(&p, &dui, x, y + 9.0, &cx.s.modal.err.clone(), sans(12.5), DANGER_TEXT);
        }
        y += 18.0 + 18.0;
    }
    let yes = Btn::cta(spec.yes.clone()).tone(if spec.danger { DANGER } else { COPPER });
    let (go, _) = buttons(&mut dui, cx, rect.right() - 24.0, y + 17.0, spec.no, yes);
    if (go || enter) && !cx.s.ov_out {
        cx.fx.push(spec.go);
    }
}

fn run(ui: &mut Ui, cx: &mut Cx, screen: Rect, pa: f32, pdy: f32, enter: bool) {
    let wd = 480.0f32.min(screen.width() - 48.0);
    let iw = wd - 48.0;
    let perr = cx.s.run.perr.clone();
    let perr_g = (!perr.is_empty()).then(|| w::g_wrap(ui, &perr, sans(12.5), TEXT, iw - 28.0 - 17.0, 18.75));
    let perr_h = perr_g.as_ref().map_or(0.0, |g| g.size().y + 22.0 + 18.0);
    let h = 24.0 + 24.0 + 6.0 + 19.0 + 18.0 + (17.0 + 6.0 + 36.0 + 6.0 + 18.0 + 6.0 + 26.0) + 18.0 + (17.0 + 6.0 + 36.0) + 18.0 + perr_h + 34.0 + 24.0;
    let rect = Rect::from_center_size(screen.center() + vec2(0.0, pdy), vec2(wd, h));
    let mut dui = dialog(ui, rect, pa);
    let p = dui.painter().clone();
    let busy = cx.s.run.busy;

    if busy {
        // indeterminate bar along the top edge
        let ph = ((cx.now % 1.1) / 1.1) as f32;
        let e = Ease::STD.at(ph);
        let bw = wd * 0.4;
        let bx = rect.left() + (-1.0 + 3.5 * e) * bw;
        let clip = rect.shrink2(vec2(8.0, 0.0));
        p.with_clip_rect(clip).rect_filled(Rect::from_min_size(pos2(bx, rect.top()), vec2(bw, 2.0)), 0.0, COPPER);
        cx.ambient = true;
    }

    let x = rect.left() + 24.0;
    let mut y = rect.top() + 24.0;
    let tg = w::g_ls(&dui, "Run an image", semibold(17.0), TEXT, -0.17);
    w::put(&p, &tg, pos2(x, y + 12.0), Align2::LEFT_CENTER);
    y += 24.0 + 6.0;
    w::text(&p, &dui, x, y + 9.5, "Creates a container from the image and starts it.", sans(13.5), TEXT2);
    y += 19.0 + 18.0;

    // Image
    w::text(&p, &dui, x, y + 8.5, "Image", sans(12.0), TEXT2);
    y += 17.0 + 6.0;
    let err = !cx.s.run.err.is_empty();
    let resp = field(&mut dui, cx.tw, Rect::from_min_size(pos2(x, y), vec2(iw, 36.0)), Id::new("run-image"), &mut cx.s.run.image, "nginx:1.27-alpine", err, mono(12.5));
    maybe_focus(cx.s, "run-image", &resp);
    if resp.changed() {
        cx.s.run.err.clear();
    }
    y += 36.0 + 6.0;
    if err {
        w::text(&p, &dui, x, y + 9.0, &cx.s.run.err.clone(), sans(12.5), DANGER_TEXT);
    }
    y += 18.0 + 6.0;
    let mut px = x;
    for pick in RUN_PICKS {
        let b = Btn::ghost(pick).h(26.0).pad(10.0).radius(7.0).font(mono(11.5)).color(TEXT2);
        let (r, rr) = b.at_left(&mut dui, px, y + 13.0, Id::new(("pick", pick)));
        if r.clicked() {
            cx.s.run.image = pick.to_string();
            cx.s.run.err.clear();
        }
        px = rr.right() + 8.0;
    }
    y += 26.0 + 18.0;

    // Name and ports
    let col = (iw - 14.0) / 2.0;
    w::text(&p, &dui, x, y + 8.5, "Name (optional)", sans(12.0), TEXT2);
    w::text(&p, &dui, x + col + 14.0, y + 8.5, "Mac port to container port", sans(12.0), TEXT2);
    y += 17.0 + 6.0;
    let r = field(&mut dui, cx.tw, Rect::from_min_size(pos2(x, y), vec2(col, 36.0)), Id::new("run-name"), &mut cx.s.run.name, "from the image", false, mono(12.5));
    if r.changed() {
        cx.s.run.perr.clear();
    }
    let arrow_w = w::text_w(&dui, "→", sans(13.5));
    let pw = (col - arrow_w - 16.0) / 2.0;
    let hx = x + col + 14.0;
    let r = field(&mut dui, cx.tw, Rect::from_min_size(pos2(hx, y), vec2(pw, 36.0)), Id::new("run-host"), &mut cx.s.run.host, "8080", false, mono(12.5));
    if r.changed() {
        cx.s.run.perr.clear();
    }
    w::text(&p, &dui, hx + pw + 8.0, y + 18.0, "→", sans(13.5), MUTED);
    let r = field(&mut dui, cx.tw, Rect::from_min_size(pos2(hx + pw + 16.0 + arrow_w, y), vec2(pw, 36.0)), Id::new("run-port"), &mut cx.s.run.port, "80", false, mono(12.5));
    if r.changed() {
        cx.s.run.perr.clear();
    }
    y += 36.0 + 18.0;

    if let Some(g) = perr_g {
        let r = Rect::from_min_size(pos2(x, y), vec2(iw, g.size().y + 22.0));
        w::card(&p, r, 10.0, ERR_BG, ERR_EDGE);
        w::small_dot(&p, pos2(x + 14.0 + 3.5, y + 11.0 + 9.5), DANGER);
        p.galley(pos2(x + 14.0 + 7.0 + 10.0, y + 11.0), g.clone(), TEXT);
        y += g.size().y + 22.0 + 18.0;
    }

    let yes = Btn::cta(if !busy { "Run" } else if cx.s.is_live() { "Starting" } else { "Pulling" }).spin(busy);
    if busy {
        cx.ambient = true;
    }
    let (go, _) = buttons(&mut dui, cx, rect.right() - 24.0, y + 17.0, "Cancel", yes);
    if (go || enter) && !cx.s.ov_out {
        cx.fx.push(Box::new(|s| s.run_go()));
    }
}

fn machine(ui: &mut Ui, cx: &mut Cx, screen: Rect, pa: f32, pdy: f32, enter: bool) {
    let wd = 480.0f32.min(screen.width() - 48.0);
    let iw = wd - 48.0;
    let (tg, bg) = title_block(ui, "New machine", "A full Linux system with its own files and services, running next to your containers.", iw);
    let cols = (((iw + 10.0) / (132.0 + 10.0)).floor() as usize).clamp(1, DISTROS.len());
    let rows = DISTROS.len().div_ceil(cols);
    let grid_h = rows as f32 * 48.0 + (rows - 1) as f32 * 10.0;
    let h = 24.0 + tg.size().y + 6.0 + bg.size().y + 18.0 + grid_h + 18.0 + 17.0 + 6.0 + 36.0 + 18.0 + 34.0 + 24.0;
    let rect = Rect::from_center_size(screen.center() + vec2(0.0, pdy), vec2(wd, h));
    let mut dui = dialog(ui, rect, pa);
    let p = dui.painter().clone();
    let x = rect.left() + 24.0;
    let mut y = rect.top() + 24.0;
    p.galley(pos2(x, y), tg.clone(), TEXT);
    y += tg.size().y + 6.0;
    p.galley(pos2(x, y), bg.clone(), TEXT2);
    y += bg.size().y + 18.0;

    let cell_w = (iw - 10.0 * (cols - 1) as f32) / cols as f32;
    for (i, (_, label, mono_l)) in DISTROS.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x + (i % cols) as f32 * (cell_w + 10.0), y + (i / cols) as f32 * 58.0), vec2(cell_w, 48.0));
        let resp = dui.interact(r, Id::new(("distro", i)), Sense::click());
        if resp.hovered() {
            dui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let on = cx.s.mk.pick == i;
        let bd = cx.tw.c(("distro-b", i), if on { COPPER } else { BTN_EDGE_SOFT }, 0.2);
        let ic = cx.tw.c(("distro-c", i), if on { COPPER } else { TEXT2 }, 0.2);
        w::card(&p, r, 10.0, FIELD, bd);
        let mc = pos2(r.left() + 12.0 + 13.0, r.center().y);
        p.circle_filled(mc, 13.0, SEG_ON);
        let mg = w::g(&dui, mono_l, mono(11.0), ic);
        w::put(&p, &mg, mc, Align2::CENTER_CENTER);
        let lx = mc.x + 13.0 + 10.0;
        let lg = w::g_wrap(&dui, label, sans(13.0), TEXT, r.right() - 12.0 - lx, 15.0);
        w::put(&p, &lg, pos2(lx, mc.y), Align2::LEFT_CENTER);
        if resp.clicked() {
            cx.s.mk.pick = i;
        }
    }
    y += grid_h + 18.0;
    w::text(&p, &dui, x, y + 8.5, "Name", sans(12.0), TEXT2);
    y += 17.0 + 6.0;
    let ph = DISTROS[cx.s.mk.pick].0;
    let resp = field(&mut dui, cx.tw, Rect::from_min_size(pos2(x, y), vec2(iw, 36.0)), Id::new("mk-name"), &mut cx.s.mk.name, ph, false, mono(12.5));
    maybe_focus(cx.s, "mk-name", &resp);
    y += 36.0 + 18.0;
    let busy = cx.s.mk.busy;
    let yes = Btn::cta(if busy { "Creating" } else { "Create" }).spin(busy);
    if busy {
        cx.ambient = true;
    }
    let (go, _) = buttons(&mut dui, cx, rect.right() - 24.0, y + 17.0, "Cancel", yes);
    if (go || enter) && !cx.s.ov_out {
        cx.fx.push(Box::new(|s| s.make_machine()));
    }
}

fn palette(ui: &mut Ui, cx: &mut Cx, screen: Rect, pa: f32, pdy: f32, enter: bool) {
    let (up, down) = ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::ArrowUp), i.consume_key(Modifiers::NONE, Key::ArrowDown)));
    let mut cmds = cx.s.pal_filtered();
    let n = cmds.len();
    if down {
        cx.s.pal.i = (cx.s.pal.i + 1).min(n.saturating_sub(1));
    }
    if up {
        cx.s.pal.i = cx.s.pal.i.saturating_sub(1);
    }
    let pi = cx.s.pal.i.min(n.saturating_sub(1));
    let wd = 560.0f32.min(screen.width() - 48.0);
    let none_g = (n == 0).then(|| {
        let mut j = w::job(&format!("Nothing matches “{}”. Try a page, a container name or an action.", cx.s.pal.q.trim()), sans(13.5), MUTED, 0.0);
        j.wrap.max_width = wd - 16.0 - 28.0;
        j.halign = egui::Align::Center;
        ui.fonts(|f| f.layout_job(j))
    });
    let list_h = if n == 0 { none_g.as_ref().map_or(0.0, |g| g.size().y) + 44.0 } else { n as f32 * 38.0 + (n - 1) as f32 * 2.0 };
    let h = 52.0 + 16.0 + list_h;
    let rect = Rect::from_min_size(pos2(screen.center().x - wd / 2.0, screen.top() + 96.0 + pdy), vec2(wd, h));
    let mut dui = dialog(ui, rect, pa);
    let p = dui.painter().clone();
    let cy = rect.top() + 26.0;
    icons::paint(&dui, Rect::from_center_size(pos2(rect.left() + 18.0 + 8.0, cy), vec2(16.0, 16.0)), icons::SEARCH, Ink::Stroke(1.4), MUTED);
    let esc_w = w::text_w(&dui, "esc", mono(11.0)) + 12.0;
    let esc = Rect::from_min_size(pos2(rect.right() - 18.0 - esc_w, cy - 9.5), vec2(esc_w, 19.0));
    w::edge(&p, esc, 5.0, NAV_ON_EDGE);
    w::put(&p, &w::g(&dui, "esc", mono(11.0), MUTED), esc.center(), Align2::CENTER_CENTER);
    let input = Rect::from_min_max(pos2(rect.left() + 18.0 + 16.0 + 10.0, rect.top() + 8.0), pos2(esc.left() - 10.0, rect.top() + 44.0));
    let te = egui::TextEdit::singleline(&mut cx.s.pal.q)
        .id(Id::new("palette"))
        .frame(false)
        .font(sans(15.0))
        .text_color(TEXT)
        .hint_text(egui::RichText::new("Type a command or a name").color(MUTED).font(sans(15.0)))
        .desired_width(input.width())
        .min_size(input.size())
        .vertical_align(egui::Align::Center)
        .margin(egui::Margin::ZERO);
    let resp = dui.put(input, te);
    maybe_focus(cx.s, "palette", &resp);
    if resp.changed() {
        cx.s.pal.i = 0;
    }
    p.line_segment([pos2(rect.left(), rect.top() + 52.5), pos2(rect.right(), rect.top() + 52.5)], Stroke::new(1.0_f32, NAV_ON_EDGE));

    let mut y = rect.top() + 52.0 + 8.0;
    let mut chosen = None;
    for (i, c) in cmds.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 8.0, y), vec2(wd - 16.0, 38.0));
        let resp = dui.interact(r, Id::new(("pal", i)), Sense::click());
        if resp.hovered() {
            dui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let on = i == pi;
        if on || resp.hovered() {
            w::fill(&p, r, 9.0, MENU_HOVER);
        }
        w::text(&p, &dui, r.left() + 12.0, r.center().y, &c.label, sans(13.5), if on { TEXT } else { CODE });
        w::text_right(&p, &dui, r.right() - 12.0, r.center().y, c.hint, sans(12.0), MUTED);
        if resp.clicked() {
            chosen = Some(i);
        }
        y += 40.0;
    }
    if let Some(g) = none_g {
        p.galley(pos2(rect.center().x, y + 22.0), g, MUTED);
    }
    if enter && n > 0 {
        chosen = Some(pi);
    }
    if let Some(i) = chosen {
        if !cx.s.ov_out {
            let cmd = cmds.swap_remove(i);
            cx.fx.push(Box::new(move |s| s.pal_run(cmd)));
        }
    }
}

fn toast(ctx: &Context, cx: &mut Cx) {
    let Some(t) = &cx.s.toast else { return };
    let (msg, act, at) = (t.msg.clone(), t.act.clone(), t.at);
    let screen = ctx.screen_rect();
    let el = (cx.now - at) as f32;
    let e = Ease::POP.at((el / 0.35).min(1.0));
    if el < 0.4 {
        cx.tw.busy = true;
    }
    Area::new(Id::new("toast")).order(Order::Tooltip).fixed_pos(screen.min).constrain(false).interactable(true).show(ctx, |ui| {
        let mg = w::g(ui, &msg, sans(13.5), TEXT);
        let btn = (!act.is_empty()).then(|| Btn::plain(act.clone()).h(28.0).pad(10.0).radius(7.0).font(semibold(13.5)).color(COPPER));
        let bw = btn.as_ref().map_or(0.0, |b| b.width(ui) + 12.0);
        let wd = 16.0 + mg.size().x + 8.0 + bw + 8.0;
        let rect = Rect::from_min_size(pos2(screen.center().x - wd / 2.0, screen.bottom() - 22.0 - 42.0 + 18.0 * (1.0 - e)), vec2(wd, 42.0));
        let mut tui = ui.new_child(UiBuilder::new().max_rect(rect));
        tui.multiply_opacity(e.clamp(0.0, 1.0));
        let p = tui.painter().clone();
        w::shadow(&p, rect, 12.0, 40.0, 16.0, rgba(0, 0, 0, 0.45));
        w::card(&p, rect, 12.0, TOAST, TOAST_EDGE);
        w::put(&p, &mg, pos2(rect.left() + 16.0, rect.center().y), Align2::LEFT_CENTER);
        if let Some(b) = btn {
            let (r, _) = b.at_right(&mut tui, rect.right() - 8.0, rect.center().y, Id::new("toast-act"));
            if r.clicked() {
                cx.fx.push(Box::new(|s| s.undo_last()));
            }
        }
        // the line drains over 4.2 s
        let k = 1.0 - ((cx.now - at) as f32 / 4.2).clamp(0.0, 1.0);
        if k > 0.0 {
            let lw = (rect.width() - 12.0) * k;
            p.rect_filled(Rect::from_min_size(pos2(rect.left() + 6.0, rect.bottom() - 2.0), vec2(lw, 2.0)), 1.0, COPPER);
            cx.ambient = true;
        }
        ui.advance_cursor_after_rect(rect);
    });
}
