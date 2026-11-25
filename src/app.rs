//! The eframe application: frame loop, shortcuts, onboarding handoff.

use std::path::PathBuf;
use std::time::Duration;

use egui::{Id, Key, Modifiers, ViewportCommand};

use crate::live;
use crate::onboarding::{self, Onb};
use crate::sim::{Ov, Preset, State};
use crate::theme;
use crate::tween::Tweens;
use crate::ui::{self, Cx};

#[derive(Clone, Debug, Default)]
pub struct Args {
    pub preset: Preset,
    pub onboarding: bool,
    pub step: usize,
    pub screenshot: Option<PathBuf>,
    pub wait: f64,
    /// Use the simulated engine instead of real Docker.
    pub demo: bool,
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

        self.s.tick(now);
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

/// The app icon, drawn in code: the Patina mark on a dark rounded square.
pub fn app_icon(n: usize) -> egui::IconData {
    let mut rgba = vec![0u8; n * n * 4];
    let c = n as f32 / 2.0;
    let r_disc = n as f32 * 0.31;
    let half = n as f32 * 0.41;
    let rad = n as f32 * 0.09;
    let bg = theme::SIDEBAR.to_array();
    let cu = theme::COPPER.to_array();
    let vg = theme::VERDIGRIS.to_array();
    for y in 0..n {
        for x in 0..n {
            let mut acc = [0f32; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0 - c;
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0 - c;
                    // rounded square
                    let qx = (px.abs() - (half - rad)).max(0.0);
                    let qy = (py.abs() - (half - rad)).max(0.0);
                    if qx * qx + qy * qy > rad * rad {
                        continue;
                    }
                    let col = if px * px + py * py <= r_disc * r_disc {
                        if px + py > 0.0 {
                            vg
                        } else {
                            cu
                        }
                    } else {
                        bg
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
