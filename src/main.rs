//! Patina: containers and Linux machines for your Mac.
//!
//! A native Rust desktop app (egui/eframe) built from the Patina design.
//! The engine is simulated; see `sim.rs`.

mod app;
mod engine;
mod icons;
mod onboarding;
mod sim;
mod theme;
mod tween;
mod ui;
mod widgets;

use std::sync::Arc;

use app::{App, Args};

const HELP: &str = "\
Patina: containers and Linux machines for your Mac

USAGE:
    patina [OPTIONS]

OPTIONS:
    --view <name>       containers | images | volumes | networks | machines | kubernetes | settings
    --pane <name>       general | resources | network | engine | about   (with --view settings)
    --scenario <name>   default | empty | loading | stopped | failed | trouble | update | diskfull | offline
    --show <name>       run | menu | delete | toast | palette | pull | blocked | prune | volume | machine
    --phase <name>      invalid | conflict | busy | nomatch | on | dirty
    --onboarding        show the first-run flow (also shown automatically on first launch)
    --step <1-4>        start the first-run flow on this step
    --screenshot <png>  save a screenshot of the window and quit (with --wait <seconds>)
    --write-icon <png>  write the 1024px app icon and quit
    -h, --help          print this help
";

fn parse_args() -> Args {
    let mut a = Args { wait: 1.5, step: 1, ..Default::default() };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || it.next().unwrap_or_default();
        match flag.as_str() {
            "--view" => a.preset.view = val(),
            "--pane" => a.preset.pane = val(),
            "--scenario" => a.preset.scenario = val(),
            "--show" => a.preset.show = val(),
            "--phase" => a.preset.phase = val(),
            "--onboarding" => a.onboarding = true,
            "--step" => a.step = val().parse().unwrap_or(1),
            "--screenshot" => a.screenshot = Some(val().into()),
            "--wait" => a.wait = val().parse().unwrap_or(1.5),
            "--write-icon" => {
                let path = val();
                let icon = app::app_icon(1024);
                match app::save_rgba_png(std::path::Path::new(&path), &icon.rgba, icon.width, icon.height) {
                    Ok(()) => std::process::exit(0),
                    Err(e) => {
                        eprintln!("could not write {path}: {e}");
                        std::process::exit(1);
                    }
                }
            }
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown option: {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }
    a
}

fn engine_cli(args: &[String]) -> i32 {
    use engine::{control, setup};
    let report = |f: f64, note: &str| {
        if f >= 0.0 {
            eprint!("\r  {:>3.0}%  {note:<40}", f * 100.0);
        } else {
            eprint!("\r        {note:<40}");
        }
    };
    match args.first().map(String::as_str) {
        #[cfg(target_os = "macos")]
        Some("run") => engine::daemon::run(),
        Some("setup") => {
            eprintln!("Installing the docker and compose commands");
            if let Err(e) = setup::install_cli(&report) {
                eprintln!("\n{e}");
                return 1;
            }
            eprintln!("\nCreating the disk image");
            if let Err(e) = setup::create_disk(&report) {
                eprintln!("\n{e}");
                return 1;
            }
            eprintln!("\nDone.");
            0
        }
        Some("start") => match control::start() {
            Ok(()) => {
                eprintln!("Starting. Follow along with `patina engine status`.");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        Some("stop") => {
            if !control::stop() {
                eprintln!("The engine isn't running.");
                return 0;
            }
            for _ in 0..80 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if control::status().is_none() {
                    eprintln!("Stopped.");
                    return 0;
                }
            }
            eprintln!("The engine is still shutting down.");
            1
        }
        Some("status") => match control::status() {
            Some(s) => {
                println!("{}: {} ({:.0} s)", s.state, s.message, s.uptime);
                0
            }
            None => {
                println!("stopped{}", if setup::installed() { "" } else { " (not set up)" });
                3
            }
        },
        Some("reset") => {
            if control::status().is_some() {
                eprintln!("Stop the engine first.");
                return 1;
            }
            match setup::reset() {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: patina engine <setup|start|stop|status|reset|run>");
            2
        }
    }
}

fn main() -> eframe::Result {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("engine") {
        std::process::exit(engine_cli(&argv[2..]));
    }
    let args = parse_args();
    let show_onboarding = args.onboarding || (args.screenshot.is_none() && !app::is_onboarded() && args.preset.show.is_empty());
    let (size, min, resizable) = if show_onboarding {
        (app::ONB_SIZE, app::ONB_SIZE, false)
    } else {
        (app::MAIN_SIZE, app::MAIN_MIN, true)
    };
    let viewport = egui::ViewportBuilder::default()
        .with_title("Patina")
        .with_app_id("patina")
        .with_inner_size(size)
        .with_min_inner_size(min)
        .with_resizable(resizable)
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_icon(Arc::new(app::app_icon(256)));
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    eframe::run_native("Patina", options, Box::new(move |cc| Ok(Box::new(App::new(cc, args, show_onboarding)))))
}
