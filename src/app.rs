//! The eframe application: frame loop, shortcuts, onboarding handoff.

use std::path::PathBuf;
use std::time::Duration;

use egui::{Frame, Id, Key, Modifiers, ViewportBuilder, ViewportCommand, ViewportId};

use crate::live;
use crate::menubar::{self, Anchor, POPOVER_SIZE, POPOVER_TITLE};
use crate::onboarding::{self, Onb};
use crate::sim::{Engine, Ov, Preset, State};
use crate::theme;
use crate::tween::Tweens;
use crate::ui::{self, popover, Cx};

#[derive(Clone, Debug, Default)]
pub struct Args {
    pub preset: Preset,
    pub onboarding: bool,
    pub step: usize,
    pub screenshot: Option<PathBuf>,
    pub wait: f64,
    /// Use the simulated engine instead of real Docker.
    pub demo: bool,
    /// Draw the menu bar popover in the main window (for screenshots).
    pub popover: bool,
}

/// The open menu bar popover.
struct Pop {
    anchor: Anchor,
    opened: f64,
    /// Frames drawn so far (native styling happens on the first few).
    frames: u32,
    /// It has had keyboard focus; losing it closes the popover.
    focused: bool,
}

struct Shot {
    path: PathBuf,
    at: f64,
    sent: bool,
}

pub struct App {
    s: State,
    tw: Tweens,
    onb: Option<Onb>,
    started: f64,
    preset: Preset,
    shot: Option<Shot>,
    demo: bool,
    /// Draw only the popover, in the main window (`--popover`).
    popover_only: bool,
    /// The status item is in the menu bar.
    tray: bool,
    pop: Option<Pop>,
    /// When the popover last closed because it lost focus.
    pop_blurred: f64,
    /// The main window is closed; Patina lives in the menu bar.
    hidden: bool,
    /// Quit was chosen: let the next close request through.
    quitting: bool,
    /// The menu bar setting before the engine reports its config.
    menubar_default: bool,
}

pub const MAIN_SIZE: [f32; 2] = [1360.0, 860.0];
pub const MAIN_MIN: [f32; 2] = [1120.0, 720.0];
pub const ONB_SIZE: [f32; 2] = [880.0, 600.0];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args, show_onboarding: bool) -> App {
        theme::install_fonts(&cc.egui_ctx);
        theme::install_style(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let now = cc.egui_ctx.input(|i| i.time);
        let s = if args.demo {
            State::new(args.preset.clone(), now)
        } else {
            State::new_live(live::spawn(cc.egui_ctx.clone()), args.preset.clone(), now)
        };
        App {
            demo: args.demo,
            s,
            tw: Tweens::default(),
            onb: show_onboarding.then(|| Onb::new(now, args.step.max(1), !args.demo, Some(cc.egui_ctx.clone()))),
            started: now,
            preset: args.preset.clone(),
            shot: args.screenshot.map(|path| Shot { path, at: now + args.wait, sent: false }),
            popover_only: args.popover,
            tray: false,
            pop: None,
            pop_blurred: f64::NEG_INFINITY,
            hidden: false,
            quitting: false,
            menubar_default: args.demo || crate::engine::config::load().menubar,
        }
    }

    fn menubar_wanted(&self) -> bool {
        if self.shot.is_some() || self.popover_only {
            return false;
        }
        if self.s.is_live() {
            self.s.cfg.get("menubar").map_or(self.menubar_default, |v| v == "true")
        } else {
            self.s.set.menubar
        }
    }

    /// Bring the main window back (from the menu bar, or to the front).
    fn show_main(&mut self, ctx: &egui::Context, now: f64) {
        self.pop = None;
        menubar::set_dock(true);
        if self.hidden {
            self.hidden = false;
            self.started = now;
            self.s.view_at = now;
        }
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Focus);
        menubar::activate();
        ctx.request_repaint();
    }

    fn close_pop(&mut self, blurred: bool, now: f64) {
        menubar::trace(if blurred { "popover closed: lost focus" } else { "popover closed" });
        self.pop = None;
        if blurred {
            self.pop_blurred = now;
        } else if self.hidden {
            // Nothing of Patina's is on screen: hand the keyboard back.
            menubar::hide_app();
        }
    }

    /// The status item, its popover, and closing the window to the menu bar.
    fn menu_bar(&mut self, ctx: &egui::Context, now: f64) {
        let want = self.menubar_wanted();
        if want != self.tray {
            if want {
                self.tray = menubar::install(ctx);
            } else {
                menubar::remove();
                self.tray = false;
                self.pop = None;
                if self.hidden {
                    self.show_main(ctx, now);
                }
            }
        }
        if !self.tray {
            return;
        }
        menubar::set_running(matches!(self.s.engine, Engine::Running | Engine::Ready));

        for ev in menubar::take_events() {
            match ev {
                menubar::Event::Toggle(anchor) => {
                    menubar::trace(&format!("toggle at {:?} (open: {})", anchor.native, self.pop.is_some()));
                    if self.pop.is_some() {
                        self.close_pop(false, now);
                    } else if now - self.pop_blurred > 0.35 {
                        // A click on the status item can arrive just after the
                        // popover closed for losing focus; don't reopen it then.
                        // Patina is activated once the popover is on screen
                        // (see `style_popover`), so macOS doesn't switch Spaces.
                        self.pop = Some(Pop { anchor, opened: now, frames: 0, focused: false });
                    }
                }
                menubar::Event::Open => self.show_main(ctx, now),
                menubar::Event::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
                }
                menubar::Event::Activated => {
                    // Opened again (Finder, Spotlight) while living in the menu bar.
                    if self.hidden && self.pop.is_none() {
                        self.show_main(ctx, now);
                    }
                }
            }
        }

        // Closing the window keeps Patina running in the menu bar.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.hidden = true;
            menubar::set_dock(false);
        }

        let Some(pop) = &mut self.pop else { return };
        let t0 = pop.opened;
        let builder = ViewportBuilder::default()
            .with_title(POPOVER_TITLE)
            .with_inner_size(POPOVER_SIZE)
            .with_position(pop.anchor.pos)
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            // Made hidden: shown on the current Space once it may join every
            // Space, including a full-screen app's.
            .with_visible(false);
        let (out, fx, ambient, focused, esc) = ctx.show_viewport_immediate(ViewportId::from_hash_of("patina-popover"), builder, |ctx, _| {
            let mut cx = Cx { s: &mut self.s, tw: &mut self.tw, fx: Vec::new(), now, started: t0, ambient: false };
            let out = egui::CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| popover::show(ui, &mut cx, t0)).inner;
            let focused = ctx.input(|i| i.focused);
            let esc = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
            (out, cx.fx, cx.ambient, focused, esc)
        });
        if pop.frames < 3 {
            menubar::style_popover((pop.frames == 0).then_some(pop.anchor));
            pop.frames += 1;
            ctx.request_repaint();
        }
        let blurred = if focused {
            pop.focused = true;
            false
        } else {
            pop.focused
        };
        let acted = !fx.is_empty();
        for f in fx {
            f(&mut self.s);
        }
        if out.open || self.s.want_setup {
            self.show_main(ctx, now);
        } else if out.close || esc {
            self.close_pop(false, now);
        } else if blurred {
            self.close_pop(true, now);
        }
        if acted {
            ctx.request_repaint();
        } else if ambient {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }

    fn handle_shot(&mut self, ctx: &egui::Context, now: f64) {
        let Some(shot) = &mut self.shot else { return };
        if !shot.sent && now >= shot.at {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));
            shot.sent = true;
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            if let Err(e) = save_png(&shot.path, &image) {
                eprintln!("screenshot failed: {e}");
            } else {
                eprintln!("saved {}", shot.path.display());
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        ctx.request_repaint();
    }

    fn finish_onboarding(&mut self, ctx: &egui::Context, now: f64) {
        mark_onboarded();
        self.onb = None;
        if self.demo {
            self.s = State::new(self.preset.clone(), now);
        } else {
            self.s.installed = crate::engine::setup::installed();
            self.s.refresh();
        }
        self.s.view_at = now;
        self.started = now;
        let size = egui::vec2(MAIN_SIZE[0], MAIN_SIZE[1]);
        ctx.send_viewport_cmd(ViewportCommand::Resizable(true));
        ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(egui::vec2(MAIN_MIN[0], MAIN_MIN[1])));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
            let pos = ((monitor - size) / 2.0).max(egui::Vec2::ZERO);
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos.to_pos2()));
        }
    }
}

impl eframe::App for App {
    /// Screenshot runs open a real window; make sure no stray click or key
    /// can reach it (it may be connected to a real Docker engine).
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.shot.is_some() {
            raw.events.retain(|e| matches!(e, egui::Event::Screenshot { .. }));
            raw.modifiers = Default::default();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        self.tw.begin_frame(now);
        self.handle_shot(ctx, now);

        if self.popover_only {
            self.s.tick(now);
            let mut cx = Cx { s: &mut self.s, tw: &mut self.tw, fx: Vec::new(), now, started: self.started, ambient: false };
            let t0 = self.started;
            egui::CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| popover::show(ui, &mut cx, t0));
            let fx = std::mem::take(&mut cx.fx);
            let ambient = cx.ambient;
            let acted = !fx.is_empty();
            for f in fx {
                f(&mut self.s);
            }
            if acted || self.tw.busy {
                ctx.request_repaint();
            } else if ambient {
                ctx.request_repaint_after(Duration::from_millis(33));
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64((self.s.next_due() - now).clamp(0.0, 5.0)));
            }
            return;
        }

        if self.onb.is_none() || self.pop.is_some() {
            self.s.tick(now);
        }
        self.menu_bar(ctx, now);
        if self.hidden {
            // The window is closed to the menu bar: keep the state moving, draw nothing.
            if self.tw.busy {
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64((self.s.next_due() - now).clamp(0.0, 5.0)));
            }
            return;
        }

        if let Some(onb) = &mut self.onb {
            onb.tick(now);
            let ambient = onboarding::show(ctx, onb, &mut self.tw);
            let finished = onb.finished;
            let due = onb.next_due();
            if finished {
                self.finish_onboarding(ctx, now);
                ctx.request_repaint();
            } else if self.tw.busy {
                ctx.request_repaint();
            } else if ambient {
                ctx.request_repaint_after(Duration::from_millis(33));
            } else if let Some(due) = due {
                ctx.request_repaint_after(Duration::from_secs_f64((due - now).max(0.0)));
            }
            return;
        }

        if self.s.want_setup && !self.demo {
            self.s.want_setup = false;
            self.onb = Some(Onb::new(now, 2, true, Some(ctx.clone())));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(egui::vec2(ONB_SIZE[0], ONB_SIZE[1])));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(ONB_SIZE[0], ONB_SIZE[1])));
            ctx.send_viewport_cmd(ViewportCommand::Resizable(false));
            ctx.request_repaint();
            return;
        }

        // Global shortcuts.
        let s = &mut self.s;
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
            match s.ov {
                None => s.open_pal(),
                Some(Ov::Palette) => s.close_ov(),
                _ => {}
            }
        }
        if s.ov.is_none() && s.menu.is_some() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            s.menu = None;
        }
        // Escape leaves the log page, unless it's clearing the filter's focus.
        let filtering = ctx.memory(|m| m.has_focus(Id::new("page-q")));
        if s.ov.is_none() && s.page.is_some() && !filtering && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            s.close_page();
        }
        if s.ov.is_some() {
            ctx.memory_mut(|m| m.surrender_focus(Id::new("search")));
        }

        let mut cx = Cx { s: &mut self.s, tw: &mut self.tw, fx: Vec::new(), now, started: self.started, ambient: false };
        ui::side::show(ctx, &mut cx);
        ui::main_view::show(ctx, &mut cx);
        ui::overlays::show(ctx, &mut cx);
        let fx = std::mem::take(&mut cx.fx);
        let ambient = cx.ambient;
        let acted = !fx.is_empty();
        for f in fx {
            f(&mut self.s);
        }
        for url in self.s.urls.drain(..) {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }

        if acted || self.tw.busy {
            ctx.request_repaint();
        } else if ambient {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else {
            let wait = (self.s.next_due() - now).clamp(0.0, 5.0);
            ctx.request_repaint_after(Duration::from_secs_f64(wait));
        }
    }
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), Box<dyn std::error::Error>> {
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    save_rgba_png(path, &bytes, image.size[0] as u32, image.size[1] as u32)
}

pub fn save_rgba_png(path: &std::path::Path, rgba: &[u8], width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}

fn marker_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/Patina/onboarded"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Patina").join("onboarded"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config"))).map(|c| c.join("patina/onboarded"))
    }
}

pub fn is_onboarded() -> bool {
    marker_path().is_some_and(|p| p.exists())
}

fn mark_onboarded() {
    if let Some(p) = marker_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, b"1\n");
    }
}

/// The app icon, drawn in code: the Patina mark on a warm charcoal rounded
/// square, darkening toward the bottom, with a lighter hairline edge so it
/// holds its shape on a dark Dock or menu.
pub fn app_icon(n: usize) -> egui::IconData {
    let mut rgba = vec![0u8; n * n * 4];
    let c = n as f32 / 2.0;
    let r_disc = n as f32 * 0.31;
    let half = n as f32 * 0.41;
    let rad = n as f32 * 0.09;
    let edge = (n as f32 * 0.006).max(1.0);
    let (top, bottom) = ([54.0, 49.0, 44.0], [36.0, 33.0, 30.0]);
    let rim = [78u8, 72, 65, 255];
    let cu = theme::COPPER.to_array();
    let vg = theme::VERDIGRIS.to_array();
    for y in 0..n {
        for x in 0..n {
            let mut acc = [0f32; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0 - c;
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0 - c;
                    // rounded square: distance inside its edge
                    let qx = (px.abs() - (half - rad)).max(0.0);
                    let qy = (py.abs() - (half - rad)).max(0.0);
                    let out = (qx * qx + qy * qy).sqrt() - rad;
                    let inside = if qx > 0.0 || qy > 0.0 { -out } else { (half - px.abs()).min(half - py.abs()) };
                    if out > 0.0 {
                        continue;
                    }
                    let col = if px * px + py * py <= r_disc * r_disc {
                        if px + py > 0.0 {
                            vg
                        } else {
                            cu
                        }
                    } else if inside < edge {
                        rim
                    } else {
                        let t = ((py + half) / (2.0 * half)).clamp(0.0, 1.0);
                        let m = |k: usize| (top[k] + (bottom[k] - top[k]) * t) as u8;
                        [m(0), m(1), m(2), 255]
                    };
                    for k in 0..3 {
                        acc[k] += col[k] as f32;
                    }
                    acc[3] += 255.0;
                }
            }
            let i = (y * n + x) * 4;
            let cov = acc[3] / 16.0;
            for k in 0..3 {
                rgba[i + k] = if cov > 0.0 { (acc[k] / (acc[3] / 255.0)).round() as u8 } else { 0 };
            }
            rgba[i + 3] = cov.round() as u8;
        }
    }
    egui::IconData { rgba, width: n as u32, height: n as u32 }
}
