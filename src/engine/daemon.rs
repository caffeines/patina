//! `patina engine run`: the helper process that owns the VM.
//!
//! The main thread runs the Virtualization framework (via `dispatch_main`).
//! A Tokio runtime on another thread serves:
//! - `~/.patina/run/docker.sock`, piped to the guest's Docker over vsock,
//! - `~/.patina/run/engine.sock`, a one-line protocol (`status`, `stop`),
//! - localhost listeners for every published container port,
//! - `<machine>.patina.local` names for running machines, on this Mac,
//! - a monitor that notices when Docker is ready and when the VM stops,
//! - the memory balloon, which keeps the guest's file cache from growing the
//!   VM to its full memory ([`super::balloon`]).

use std::collections::{HashMap, HashSet};
use std::os::fd::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bollard::query_parameters::ListContainersOptionsBuilder;
use bollard::{Docker, API_DEFAULT_VERSION};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::task::JoinHandle;

use super::balloon::{self, Balloon};
use super::control::Status;
use super::{config, paths, setup, vm, PORT_DOCKER, PORT_FORWARD, PORT_MEMORY};

struct Shared {
    status: Mutex<Status>,
    started: Instant,
    stopping: AtomicBool,
    was_running: AtomicBool,
}

impl Shared {
    fn set(&self, state: &str, message: &str) {
        let mut s = self.status.lock().unwrap();
        if s.state != state || s.message != message {
            log(&format!("{state}: {message}"));
        }
        s.state = state.into();
        s.message = message.into();
    }
    fn state(&self) -> String {
        self.status.lock().unwrap().state.clone()
    }
    fn snapshot(&self) -> Status {
        let mut s = self.status.lock().unwrap().clone();
        s.uptime = self.started.elapsed().as_secs_f64();
        s
    }
}

fn log(msg: &str) {
    let t = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    println!("{t} {msg}");
}

fn cleanup() {
    for p in [paths::docker_sock(), paths::control_sock(), paths::pid_file()] {
        let _ = std::fs::remove_file(p);
    }
}

pub fn run() -> i32 {
    paths::ensure();
    if super::control::status().is_some() {
        println!("The engine is already running.");
        return 0;
    }
    if !setup::installed() {
        eprintln!("Patina's engine isn't set up yet. Run `patina engine setup` first.");
        return 1;
    }
    let cfg = config::load();
    let first_boot = !paths::provisioned().exists();
    let mac = std::fs::read_to_string(paths::mac_file()).map(|s| s.trim().to_string()).unwrap_or_else(|_| "52:54:00:12:34:56".into());
    let spec = vm::Spec {
        cpus: cfg.cpus,
        memory_gb: cfg.memory_gb,
        disk: paths::disk(),
        seed: paths::seed(),
        efi: paths::efi_vars(),
        console: paths::console_log(),
        home: paths::home(),
        mac,
        rosetta: cfg.rosetta,
    };
    let memory = vm::memory_size(&spec);
    cleanup();
    let _ = std::fs::write(paths::pid_file(), std::process::id().to_string());
    log(&format!("starting Patina engine: {} CPUs, {} GB memory, first boot: {first_boot}", cfg.cpus, cfg.memory_gb));

    let shared = Arc::new(Shared {
        status: Mutex::new(Status { state: "starting".into(), message: "Starting the VM".into(), first_boot, uptime: 0.0, vm: String::new(), rev: super::control::ENGINE_REV }),
        started: Instant::now(),
        stopping: AtomicBool::new(false),
        was_running: AtomicBool::new(false),
    });

    let sh = shared.clone();
    std::thread::Builder::new()
        .name("patina-engine".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime");
            rt.block_on(serve(sh, memory));
        })
        .expect("engine thread");

    let sh = shared.clone();
    let started = vm::start(&spec, move |r| match r {
        Ok(()) => {
            sh.was_running.store(true, Ordering::SeqCst);
            let msg = if first_boot { "Installing Docker Engine. The first start takes a few minutes." } else { "Booting Linux" };
            sh.set(if first_boot { "installing" } else { "booting" }, msg);
        }
        Err(e) => {
            sh.set("error", &format!("The VM couldn't start: {e}"));
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(1500));
                cleanup();
                std::process::exit(1);
            });
        }
    });
    if let Err(e) = started {
        log(&format!("error: {e}"));
        cleanup();
        return 1;
    }
    dispatch2::dispatch_main();
}

async fn serve(sh: Arc<Shared>, memory: u64) {
    tokio::spawn(control_server(sh.clone()));
    tokio::spawn(docker_proxy());
    tokio::spawn(monitor(sh.clone()));
    tokio::spawn(forwarder(sh.clone()));
    tokio::spawn(reclaim(sh.clone(), memory));
    let sig = sh.clone();
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        shutdown(sig).await;
    });
    std::future::pending::<()>().await;
}

fn stream(fd: RawFd) -> std::io::Result<UnixStream> {
    let s = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
    s.set_nonblocking(true)?;
    UnixStream::from_std(s)
}

fn private_socket(path: &std::path::Path) -> std::io::Result<UnixListener> {
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path)?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(l)
}

async fn control_server(sh: Arc<Shared>) {
    let l = match private_socket(&paths::control_sock()) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("error: control socket: {e}"));
            return;
        }
    };
    loop {
        let Ok((s, _)) = l.accept().await else { continue };
        let sh = sh.clone();
        tokio::spawn(async move {
            let (r, mut w) = s.into_split();
            let mut lines = BufReader::new(r).lines();
            let Ok(Some(line)) = lines.next_line().await else { return };
            let reply = match line.trim() {
                "status" => serde_json::to_string(&sh.snapshot()).unwrap_or_default(),
                "stop" => {
                    tokio::spawn(shutdown(sh.clone()));
                    r#"{"ok":true}"#.to_string()
                }
                _ => r#"{"error":"unknown command"}"#.to_string(),
            };
            let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
        });
    }
}

async fn docker_proxy() {
    let l = match private_socket(&paths::docker_sock()) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("error: docker socket: {e}"));
            return;
        }
    };
    loop {
        let Ok((mut client, _)) = l.accept().await else { continue };
        tokio::spawn(async move {
            let Ok((fd, id)) = vm::connect(PORT_DOCKER).await else { return };
            if let Ok(mut guest) = stream(fd) {
                let _ = tokio::io::copy_bidirectional(&mut client, &mut guest).await;
            }
            vm::release(id);
        });
    }
}

/// Watch the VM and Docker: mark the engine ready, and exit when the VM stops.
async fn monitor(sh: Arc<Shared>) {
    let mut docker: Option<Docker> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let st = tokio::task::spawn_blocking(|| vm::on_main(vm::state)).await.unwrap_or(vm::State::None);
        {
            let mut s = sh.status.lock().unwrap();
            let now = format!("{st:?}");
            if s.vm != now {
                log(&format!("vm state: {now}"));
                s.vm = now;
            }
        }
        if sh.was_running.load(Ordering::SeqCst) && matches!(st, vm::State::Stopped | vm::State::Error) {
            log(&format!("the VM stopped ({st:?})"));
            cleanup();
            std::process::exit(if st == vm::State::Error { 1 } else { 0 });
        }
        let state = sh.state();
        if state == "booting" || state == "installing" {
            if docker.is_none() {
                docker = Docker::connect_with_unix(&paths::docker_sock().to_string_lossy(), 5, API_DEFAULT_VERSION).ok();
            }
            if let Some(d) = &docker {
                if d.ping().await.is_ok() {
                    sh.status.lock().unwrap().first_boot = false;
                    sh.set("running", "Running");
                    let d = d.clone();
                    tokio::spawn(async move {
                        match super::guest::upgrade(&d).await {
                            Ok(true) => log("upgraded the guest's channels"),
                            Ok(false) => {}
                            Err(e) => log(&format!("couldn't upgrade the guest: {e}")),
                        }
                    });
                    if config::load().set_context {
                        match tokio::task::spawn_blocking(super::integrate::use_context).await {
                            Ok(Ok(())) => log("the docker command now uses the patina context"),
                            Ok(Err(e)) => log(&format!("couldn't switch the docker context: {e}")),
                            Err(_) => {}
                        }
                    }
                }
            }
        }
    }
}

async fn shutdown(sh: Arc<Shared>) {
    if sh.stopping.swap(true, Ordering::SeqCst) {
        return;
    }
    sh.set("stopping", "Shutting down Linux");
    let asked = tokio::task::spawn_blocking(|| vm::on_main(vm::request_stop)).await.ok().and_then(|r| r.ok()).is_some();
    let deadline = Instant::now() + if asked { Duration::from_secs(30) } else { Duration::ZERO };
    loop {
        let st = tokio::task::spawn_blocking(|| vm::on_main(vm::state)).await.unwrap_or(vm::State::None);
        if matches!(st, vm::State::Stopped | vm::State::Error | vm::State::None) {
            break;
        }
        if Instant::now() >= deadline {
            log("the guest didn't shut down in time; turning the VM off");
            let _ = tokio::task::spawn_blocking(|| vm::on_main(vm::force_stop)).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    log("stopped");
    cleanup();
    std::process::exit(0);
}

/// Size the memory balloon from the guest's memory use, so the guest's file
/// cache doesn't grow the VM to its full memory. `size` is the VM's memory.
async fn reclaim(sh: Arc<Shared>, size: u64) {
    let mut balloon: Option<Balloon> = None;
    let mut failing = false;
    loop {
        tokio::time::sleep(balloon::INTERVAL).await;
        if sh.state() != "running" || sh.stopping.load(Ordering::SeqCst) {
            continue;
        }
        let stats = match guest_memory().await {
            Ok(s) => s,
            Err(e) => {
                // A VM set up by an older Patina gets the channel from
                // guest::upgrade, just after the engine starts.
                if !failing {
                    log(&format!("couldn't read the guest's memory use: {e}"));
                }
                failing = true;
                continue;
            }
        };
        failing = false;
        // Settling starts with the first reading, once Docker is up.
        let b = balloon.get_or_insert_with(|| Balloon::new(size, Instant::now()));
        let Some(target) = b.next(&stats, Instant::now()) else { continue };
        let _ = tokio::task::spawn_blocking(move || vm::on_main(move || vm::set_memory_target(target))).await;
        log(&format!("memory: the guest may use {} (it uses {})", gb(target), gb(stats.used())));
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

/// Ask the guest's `patina-memory` for its memory use.
async fn guest_memory() -> Result<balloon::Stats, String> {
    let (fd, id) = vm::connect(PORT_MEMORY).await?;
    let read = async {
        let mut s = stream(fd).map_err(|e| e.to_string())?;
        let mut text = String::new();
        s.read_to_string(&mut text).await.map_err(|e| e.to_string())?;
        balloon::Stats::parse(&text).ok_or_else(|| format!("unexpected reply: {:?}", text.trim()))
    };
    let r = tokio::time::timeout(Duration::from_secs(10), read).await.unwrap_or_else(|_| Err("no reply".into()));
    vm::release(id);
    r
}

/// Mirror published container ports onto localhost, and name running
/// machines.
async fn forwarder(sh: Arc<Shared>) {
    let mut open: HashMap<u16, Vec<JoinHandle<()>>> = HashMap::new();
    let mut docker: Option<Docker> = None;
    let mut names: Option<super::mdns::Names> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if sh.state() != "running" {
            continue;
        }
        if docker.is_none() {
            docker = Docker::connect_with_unix(&paths::docker_sock().to_string_lossy(), 10, API_DEFAULT_VERSION).ok();
        }
        let Some(d) = &docker else { continue };
        let Ok(list) = d.list_containers(Some(ListContainersOptionsBuilder::new().all(false).build())).await else { continue };
        let machines: HashMap<String, std::net::Ipv4Addr> = list
            .iter()
            .filter_map(super::machines::info)
            .filter_map(|m| Some((super::machines::domain(&m.name), m.ip.parse().ok()?)))
            .collect();
        if names.is_none() {
            names = super::mdns::Names::new();
        }
        if let Some(n) = names.as_mut() {
            if !n.set(&machines) {
                log("mDNSResponder went away; naming machines again");
                names = None;
            }
        }
        let mut want: HashSet<u16> = HashSet::new();
        if config::load().kubernetes {
            want.insert(super::kube::API_PORT);
        }
        for c in &list {
            for p in c.ports.iter().flatten() {
                let tcp = p.typ.map(|t| t.to_string()).is_none_or(|t| t == "tcp");
                if let (Some(port), true) = (p.public_port, tcp) {
                    want.insert(port);
                }
            }
        }
        open.retain(|port, handles| {
            let keep = want.contains(port);
            if !keep {
                for h in handles.iter() {
                    h.abort();
                }
                log(&format!("stopped forwarding localhost:{port}"));
            }
            keep
        });
        for port in want {
            if open.contains_key(&port) {
                continue;
            }
            let mut handles = Vec::new();
            for addr in ["127.0.0.1", "::1"] {
                handles.push(tokio::spawn(listen(addr, port)));
            }
            open.insert(port, handles);
        }
    }
}

async fn listen(addr: &'static str, port: u16) {
    let l = match TcpListener::bind((addr, port)).await {
        Ok(l) => l,
        Err(e) => {
            if addr == "127.0.0.1" {
                log(&format!("couldn't forward localhost:{port}: {e}"));
            }
            return;
        }
    };
    if addr == "127.0.0.1" {
        log(&format!("forwarding localhost:{port}"));
    }
    loop {
        let Ok((mut tcp, _)) = l.accept().await else { continue };
        tokio::spawn(async move {
            let Ok((fd, id)) = vm::connect(PORT_FORWARD).await else { return };
            if let Ok(mut guest) = stream(fd) {
                if guest.write_all(format!("{port}\n").as_bytes()).await.is_ok() {
                    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut guest).await;
                }
            }
            vm::release(id);
        });
    }
}
