//! First run: welcome, setup, bring your work over from Docker Desktop, ready.

use std::f32::consts::PI;

use egui::{pos2, vec2, Align2, Context, Frame, Id, Rect, Sense, Stroke, UiBuilder, ViewportCommand};

use crate::sim::St;
use crate::theme::*;
use crate::tween::{rise, Ease, Tweens};
use crate::widgets::{self as w, Btn};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mig {
    Idle,
    Busy,
    Done,
}

type OJob = Box<dyn FnOnce(&mut Onb)>;

pub struct Onb {
    pub step: usize,
    step_at: f64,
    setup: [u8; 3],
    setup_at: [f64; 3],
    mig: Mig,
    mp: f64,
    copied_at: f64,
    time: f64,
    started: f64,
    timers: Vec<(f64, u64, OJob)>,
    seq: u64,
    pub finished: bool,
}

const COMMAND: &str = "docker run --rm hello-world";

impl Onb {
    pub fn new(now: f64, step: usize) -> Onb {
        let step = step.clamp(1, 4);
        let mut o = Onb {
            step,
            step_at: now,
            setup: match step {
                2 => [0, 0, 0],
                s if s > 2 => [2, 2, 2],
                _ => [0, 0, 0],
            },
            setup_at: [-10.0; 3],
            mig: Mig::Idle,
            mp: 0.0,
            copied_at: -10.0,
            time: now,
            started: now,
            timers: Vec::new(),
            seq: 0,
            finished: false,
        };
        if step == 2 {
            o.run_setup();
        }
        o
    }

    fn after(&mut self, ms: u64, f: impl FnOnce(&mut Onb) + 'static) {
        self.seq += 1;
        self.timers.push((self.time + ms as f64 / 1000.0, self.seq, Box::new(f)));
    }

    pub fn next_due(&self) -> Option<f64> {
        self.timers.iter().map(|t| t.0).reduce(f64::min)
    }

    pub fn tick(&mut self, now: f64) {
        self.time = now;
        let mut due = Vec::new();
        let mut i = 0;
        while i < self.timers.len() {
            if self.timers[i].0 <= now {
                due.push(self.timers.swap_remove(i));
            } else {
                i += 1;
            }
        }
        due.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, _, f) in due {
            f(self);
        }
    }

    fn go_step(&mut self, step: usize) {
        self.step = step;
        self.step_at = self.time;
        if step == 2 {
            self.run_setup();
        }
    }

    fn run_setup(&mut self) {
        if self.setup[2] == 2 {
            return;
        }
        self.setup = [1, 0, 0];
        for i in 0..3usize {
            self.after(900 * (i as u64 + 1), move |o| {
                for j in 0..3 {
                    if j == i {
                        o.setup[j] = 2;
                    } else if j == i + 1 {
                        o.setup[j] = 1;
                    }
                }
                o.setup_at[i] = o.time;
            });
        }
    }

    fn start_import(&mut self) {
        if self.mig != Mig::Idle {
            return;
        }
        self.mig = Mig::Busy;
        self.mp = 0.08;
        for (i, v) in [0.3, 0.55, 0.8, 1.0].into_iter().enumerate() {
            self.after(500 * (i as u64 + 1), move |o| {
                o.mp = v;
                if v >= 1.0 {
                    o.mig = Mig::Done;
                }
            });
        }
    }
}

/// Draw the onboarding window. Returns true while something is animating.
pub fn show(ctx: &Context, o: &mut Onb, tw: &mut Tweens) -> bool {
    let now = o.time;
    let ambient = true;
    let mut next: Option<Box<dyn FnOnce(&mut Onb)>> = None;
    egui::CentralPanel::default().frame(Frame::new().fill(BG)).show(ctx, |ui| {
        let full = ui.max_rect();
        let drag = ui.interact(Rect::from_min_size(full.min, vec2(full.width(), 44.0)), Id::new("onb-drag"), Sense::drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let p = ui.painter().clone();

        // ---- aside
        let aside = Rect::from_min_size(full.min, vec2(300.0, full.height()));
        p.rect_filled(aside, 0.0, SIDEBAR);
        p.line_segment([pos2(aside.right() - 0.5, aside.top()), pos2(aside.right() - 0.5, aside.bottom())], Stroke::new(1.0_f32, HAIRLINE));
        let done = o.setup.iter().filter(|v| **v == 2).count() as f32;
        let frac = match o.step {
            1 => 0.5,
            2 => 0.5 + done * 0.08,
            3 => 0.76 + o.mp as f32 * 0.12,
            _ => 1.0,
        };
        let frac = tw.f("onb-pie", frac, 0.9, Ease::PIE);
        let (ia, idy, irun) = rise(now, o.started, 0.0, 0.7, 10.0);
        if irun {
            tw.busy = true;
        }
        let mut ip = p.clone();
        ip.multiply_opacity(ia);
        let ill_top = aside.top() + 20.0 + 12.0 + 26.0;
        let c = pos2(aside.center().x, ill_top + 110.0 + idy);
        // glow ring
        let ph = ((now % 3.4) / 3.4) as f32;
        if ph < 0.7 {
            let e = Ease::EASE_OUT.at(ph / 0.7);
            ip.circle_stroke(c, 65.25 * (1.0 + 0.38 * e), Stroke::new(1.5_f32, VERDIGRIS.gamma_multiply(0.6 * (1.0 - e))));
        }
        ip.circle_filled(c, 60.0, COPPER);
        w::pie(&ip, c, 60.0, -PI / 4.0, frac * std::f32::consts::TAU, VERDIGRIS);

        let mut y = ill_top + 220.0 + 26.0;
        for (i, label) in ["Welcome", "Set up", "Bring your work", "Ready"].into_iter().enumerate() {
            let past = i + 1 < o.step || o.step == 4;
            let cur = i + 1 == o.step && o.step != 4;
            let col = tw.c(("onb-step", i), if past || cur { TEXT } else { MUTED }, 0.4);
            let dc = pos2(aside.left() + 24.0 + 5.0, y + 10.0);
            if cur {
                let ph = ((now % 2.4) / 2.4) as f32;
                if ph < 0.7 {
                    let e = Ease::EASE_OUT.at(ph / 0.7);
                    p.circle_filled(dc, 5.0 * (1.0 + 1.6 * e), COPPER.gamma_multiply(0.5 * (1.0 - e)));
                }
            }
            let (fill, edge) = if past {
                (VERDIGRIS, VERDIGRIS)
            } else if cur {
                (COPPER, COPPER)
            } else {
                (egui::Color32::TRANSPARENT, DIM)
            };
            let fill = tw.c(("onb-dot-f", i), fill, 0.7);
            let edge = tw.c(("onb-dot-e", i), edge, 0.7);
            p.circle_filled(dc, 5.0, fill);
            p.circle_stroke(dc, 4.25, Stroke::new(1.5_f32, edge));
            w::text(&p, ui, dc.x + 5.0 + 12.0, dc.y, label, sans(14.0), col);
            y += 20.0 + 14.0;
        }

        // ---- main
        let main = Rect::from_min_max(pos2(aside.right() + 56.0, full.top() + 56.0), pos2(full.right() - 56.0, full.bottom() - 32.0));
        let (sa, sdy, srun) = rise(now, o.step_at, 0.0, 0.45, 8.0);
        if srun {
            tw.busy = true;
        }
        let mut mui = ui.new_child(UiBuilder::new().max_rect(main));
        mui.multiply_opacity(sa);
        let mp = mui.painter().clone();
        let text = [
            (
                "Welcome",
                "Containers and Linux machines for your Mac",
                "Patina runs Docker containers and full Linux machines inside one small virtual machine. Setup installs the command line tools and starts the engine.",
            ),
            ("Step 2 of 4", "Setting things up", "This runs once. You can keep working while it finishes."),
            (
                "Step 3 of 4",
                "Bring your work along",
                "Docker Desktop is installed on this Mac. Patina can copy its containers, images and volumes, so you pick up where you left off.",
            ),
            ("Step 4 of 4", "Patina is ready", "The docker command now talks to Patina. Try it in a terminal."),
        ][o.step - 1];
        let mut y = main.top() + sdy;
        let kg = w::g_ls(&mui, text.0, sans(12.0), MUTED, 0.72);
        w::put(&mp, &kg, pos2(main.left(), y), Align2::LEFT_TOP);
        y += kg.size().y + 16.0;
        let tg = {
            let mut j = w::job(text.1, semibold(30.0), TEXT, -0.9);
            j.sections[0].format.line_height = Some(33.6);
            j.wrap.max_width = main.width();
            mui.fonts(|f| f.layout_job(j))
        };
        mp.galley(pos2(main.left(), y), tg.clone(), TEXT);
        y += tg.size().y + 16.0;
        let bg = w::g_wrap(&mui, text.2, sans(14.0), TEXT2, 380.0f32.min(main.width()), 20.3);
        mp.galley(pos2(main.left(), y), bg.clone(), TEXT2);
        y += bg.size().y + 16.0 + 8.0;

        match o.step {
            2 => {
                let card = Rect::from_min_size(pos2(main.left(), y), vec2(main.width(), 52.0 * 3.0));
                w::card(&mp, card, 14.0, CARD, CARD_EDGE);
                for (i, label) in ["Install the docker and compose commands", "Create the disk image", "Start the engine"].into_iter().enumerate() {
                    let top = card.top() + i as f32 * 52.0;
                    if i > 0 {
                        mp.line_segment([pos2(card.left(), top + 0.5), pos2(card.right(), top + 0.5)], Stroke::new(1.0_f32, HAIRLINE));
                    }
                    let cy = top + 26.0;
                    let dc = pos2(card.left() + 20.0 + 5.0, cy);
                    let v = o.setup[i];
                    match v {
                        2 => {
                            let ft = ((now - o.setup_at[i]) / 1.2) as f32;
                            if (0.0..1.0).contains(&ft) {
                                let e = Ease::EASE_OUT.at(ft);
                                mp.circle_filled(dc, 5.0 + 16.0 * e, mix(rgba(212, 145, 92, 0.85), rgba(111, 194, 176, 0.0), e));
                                tw.busy = true;
                            }
                            mp.circle_filled(dc, 5.0, VERDIGRIS);
                        }
                        1 => {
                            w::status_dot(&mp, tw, Id::new(("onb-task", i)), dc, St::Starting, false, -10.0);
                        }
                        _ => {
                            mp.circle_stroke(dc, 4.25, Stroke::new(1.5_f32, DIM));
                        }
                    }
                    let col = tw.c(("onb-task-c", i), if v > 0 { TEXT } else { MUTED }, 0.4);
                    w::text(&mp, &mui, dc.x + 5.0 + 16.0, cy, label, sans(14.0), col);
                    let note = match v {
                        2 => "done",
                        1 => "working",
                        _ => "",
                    };
                    w::text_right(&mp, &mui, card.right() - 20.0, cy, note, sans(12.5), MUTED);
                }
            }
            3 => {
                let card = Rect::from_min_size(pos2(main.left(), y), vec2(main.width(), 20.0 + 52.0 + 18.0 + 3.0 + 18.0 + 18.0 + 20.0));
                w::card(&mp, card, 14.0, CARD, CARD_EDGE);
                let x = card.left() + 20.0;
                let iw = card.width() - 40.0;
                let col_w = (iw - 32.0) / 3.0;
                for (i, (n, label)) in [("9", "containers"), ("14", "images"), ("6", "volumes")].into_iter().enumerate() {
                    let cx0 = x + i as f32 * (col_w + 16.0);
                    let ng = w::g_ls(&mui, n, mono(24.0), TEXT, -0.72);
                    w::put(&mp, &ng, pos2(cx0, card.top() + 20.0 + 16.0), Align2::LEFT_CENTER);
                    w::text(&mp, &mui, cx0, card.top() + 20.0 + 32.0 + 9.0, label, sans(12.5), MUTED);
                }
                let by = card.top() + 20.0 + 52.0 + 18.0;
                let fw = tw.f("onb-mig", o.mp as f32, 0.5, Ease::STD);
                let bc = tw.c("onb-mig-c", if o.mig == Mig::Done { VERDIGRIS } else { COPPER }, 0.7);
                w::bar(&mp, Rect::from_min_size(pos2(x, by), vec2(iw, 3.0)), fw, bc, TRACK);
                let note = match o.mig {
                    Mig::Done => "Imported. Docker Desktop was left as it was.",
                    Mig::Busy => "Copying images and volumes",
                    Mig::Idle => "Nothing is changed in Docker Desktop.",
                };
                w::text(&mp, &mui, x, by + 3.0 + 18.0 + 9.0, note, sans(12.5), MUTED);
            }
            4 => {
                let r = Rect::from_min_size(pos2(main.left(), y), vec2(main.width(), 46.0));
                w::card(&mp, r, 12.0, TERM, CARD_EDGE);
                let cy = r.center().y;
                let d = w::text(&mp, &mui, r.left() + 18.0, cy, "$", mono(13.0), VERDIGRIS);
                let cmd = w::text(&mp, &mui, d.right() + 8.0, cy, COMMAND, mono(13.0), TEXT);
                w::cursor_block(&mp, Rect::from_min_size(pos2(cmd.right() + 8.0, cy - 7.5), vec2(7.0, 15.0)), now);
                let copied = now - o.copied_at < 1.6;
                if copied {
                    tw.busy = true;
                }
                let (resp, _) = Btn::ghost(if copied { "Copied" } else { "Copy" }).h(30.0).pad(12.0).radius(8.0).at_right(&mut mui, r.right() - 8.0, cy, Id::new("onb-copy"));
                if resp.clicked() {
                    ctx.copy_text(COMMAND.to_string());
                    next = Some(Box::new(|o: &mut Onb| o.copied_at = o.time));
                }
            }
            _ => {}
        }

        // ---- footer
        let fy = main.bottom() - 18.0;
        let mut fui = ui.new_child(UiBuilder::new().max_rect(main));
        if o.step > 1 {
            let (r, _) = Btn::ghost("Back").h(36.0).radius(10.0).at_left(&mut fui, main.left(), fy, Id::new("onb-back"));
            if r.clicked() {
                let s = o.step - 1;
                next = Some(Box::new(move |o: &mut Onb| o.go_step(s)));
            }
        }
        let mut right = main.right();
        if o.step < 4 {
            let label = match (o.step, o.mig) {
                (1, _) => "Get started",
                (3, Mig::Done) => "Continue",
                (3, Mig::Busy) => "Importing",
                (3, Mig::Idle) => "Import",
                _ => "Continue",
            };
            let off = (o.step == 2 && done < 3.0) || (o.step == 3 && o.mig == Mig::Busy);
            let (r, rr) = Btn::cta(label).h(36.0).pad(18.0).radius(10.0).sheen(true).enabled(!off).at_right(&mut fui, right, fy, Id::new("onb-next"));
            if r.clicked() {
                let step = o.step;
                let mig = o.mig;
                next = Some(Box::new(move |o: &mut Onb| {
                    if step == 3 && mig == Mig::Idle {
                        o.start_import()
                    } else {
                        o.go_step(step + 1)
                    }
                }));
            }
            right = rr.left() - 10.0;
            if o.step == 3 && o.mig == Mig::Idle {
                let (r, _) = Btn::ghost("Skip").h(36.0).radius(10.0).at_right(&mut fui, right, fy, Id::new("onb-skip"));
                if r.clicked() {
                    next = Some(Box::new(|o: &mut Onb| o.go_step(4)));
                }
            }
        } else {
            let (r, _) = Btn::cta("Open Patina").h(36.0).pad(18.0).radius(10.0).sheen(true).at_right(&mut fui, right, fy, Id::new("onb-open"));
            if r.clicked() {
                next = Some(Box::new(|o: &mut Onb| o.finished = true));
            }
        }
    });
    if let Some(f) = next {
        f(o);
        ctx.request_repaint();
    }
    ambient
}
