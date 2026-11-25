//! Patina: containers and Linux machines for your Mac.
//!
//! A native Rust desktop app (egui/eframe) built from the Patina design.
//! Patina runs its own engine: Docker Engine in a Linux VM it boots with
//! Apple's Virtualization framework (see `engine/`). `--demo` runs the
//! simulated engine from the design instead.

mod app;
mod engine;
mod icons;
mod live;
mod menubar;
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
    --demo              use the simulated engine from the design instead of real Docker
    --onboarding        show the first-run flow (also shown automatically on first launch)
    --step <1-4>        start the first-run flow on this step
    --screenshot <png>  save a screenshot of the window and quit (with --wait <seconds>);
                        the window ignores all input while it waits
    --popover           show the menu bar popover in a window of its own (for screenshots)
    --write-icon <png>  write the 1024px app icon and quit
    -V, --version       print the version
    -h, --help          print this help

VIEW OPTIONS:
    --view <name>       containers | images | volumes | networks | builds | machines | kubernetes | settings
    --pane <name>       general | resources | network | engine | about   (with --view settings)

DEMO OPTIONS (imply --demo):
    --scenario <name>   default | empty | loading | stopped | failed | trouble | update | diskfull | offline
    --show <name>       run | menu | delete | toast | palette | pull | blocked | prune | volume | machine
                        | logs | buildlog
    --phase <name>      invalid | conflict | busy | nomatch | on | dirty
";

fn parse_args() -> Args {
    let mut a = Args { wait: 1.5, step: 1, ..Default::default() };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || it.next().unwrap_or_default();
        match flag.as_str() {
            "--demo" => a.demo = true,
            "--view" => a.preset.view = val(),
            "--pane" => a.preset.pane = val(),
            "--scenario" => a.preset.scenario = val(),
            "--show" => a.preset.show = val(),
            "--phase" => a.preset.phase = val(),
            "--onboarding" => a.onboarding = true,
            "--popover" => a.popover = true,
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
            "-V" | "--version" => {
                println!("patina {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
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
    let p = &a.preset;
    if !(p.scenario.is_empty() && p.show.is_empty() && p.phase.is_empty()) {
        a.demo = true;
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
            eprintln!("Installing docker, compose and buildx");
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
        Some("import") => {
            let Some(src) = engine::migrate::detect() else {
                eprintln!("No Docker Desktop, Colima or OrbStack with anything to import.");
                return 0;
            };
            if !src.running {
                eprintln!("{} isn't running. Start it, then run this again.", src.name);
                return 1;
            }
            eprintln!("Importing {} images, {} volumes and {} containers from {}", src.images, src.volumes, src.containers, src.name);
            match engine::migrate::run(&src, &report) {
                Ok(s) => {
                    eprintln!("\nCopied {} images, {} volumes and {} containers. Containers are stopped.", s.images, s.volumes, s.containers);
                    for f in &s.failed {
                        eprintln!("  couldn't copy {f}");
                    }
                    i32::from(!s.failed.is_empty())
                }
                Err(e) => {
                    eprintln!("\n{e}");
                    1
                }
            }
        }
        Some("kube") => {
            let on = match args.get(1).map(String::as_str) {
                Some("on") => Some(true),
                Some("off") => Some(false),
                Some("reset") => None,
                _ => {
                    eprintln!("usage: patina engine kube <on|off|reset>");
                    return 2;
                }
            };
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
            let r = rt.block_on(async {
                let d = bollard::Docker::connect_with_unix(&engine::paths::docker_sock().to_string_lossy(), 600, bollard::API_DEFAULT_VERSION)
                    .map_err(|e| e.to_string())?
                    .negotiate_version()
                    .await
                    .map_err(|_| "The engine isn't running.".to_string())?;
                eprintln!("{}", match on { Some(true) => "Turning Kubernetes on. The first time installs k3s, which takes a minute or two.", Some(false) => "Turning Kubernetes off.", None => "Resetting the cluster." });
                match on {
                    Some(true) => engine::kube::up(&d).await,
                    Some(false) => engine::kube::down(&d, false).await.map(|_| "Kubernetes is off.".into()),
                    None => {
                        engine::kube::down(&d, true).await?;
                        engine::kube::up(&d).await
                    }
                }
            });
            match r {
                Ok(msg) => {
                    eprintln!("{msg}");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
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
            eprintln!("usage: patina engine <setup|start|stop|status|import|kube|reset|run>");
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
    let first_run = if args.demo { !app::is_onboarded() } else { !engine::setup::installed() };
    let show_onboarding = !args.popover && (args.onboarding || (args.screenshot.is_none() && first_run && args.preset.show.is_empty()));
    let (size, min, resizable) = if args.popover {
        (menubar::POPOVER_SIZE, menubar::POPOVER_SIZE, false)
    } else if show_onboarding {
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
        // The popover window has rounded corners, so the GL surface needs alpha.
        .with_transparent(true)
        .with_icon(Arc::new(app::app_icon(256)));
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    eframe::run_native("Patina", options, Box::new(move |cc| Ok(Box::new(App::new(cc, args, show_onboarding)))))
}
