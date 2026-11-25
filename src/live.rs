//! The live backend: Patina's own engine.
//!
//! A background thread runs a Tokio runtime and a Docker API client pointed at
//! Patina's engine (`~/.patina/run/docker.sock`, served by the engine VM). It
//! polls containers every second, streams CPU and memory stats for running
//! containers, follows the logs of the selected one, and refreshes images,
//! volumes and networks. Engine start, stop and settings go through
//! [`crate::engine::control`]. Results come back to the UI as [`Ev`]
//! messages; the UI sends [`Cmd`] messages the other way.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use bollard::errors::Error as DockerError;
use bollard::models::{
    ContainerCreateBody, ContainerSummary, ContainerSummaryStateEnum, HostConfig, NetworkCreateRequest, PortBinding,
    VolumeCreateRequest,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptionsBuilder, ListImagesOptionsBuilder,
    ListVolumesOptions, LogsOptionsBuilder, PruneImagesOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveImageOptionsBuilder, RemoveVolumeOptions, StatsOptionsBuilder,
};
use bollard::{Docker, API_DEFAULT_VERSION};
use futures_util::StreamExt;
use tokio::sync::mpsc as tmpsc;
use tokio::task::JoinHandle;

use crate::engine::{self, config, control, integrate, paths};
use crate::sim::{Asset, Build, BuildInfo, BuildSt, Net, Page, St};

// ------------------------------------------------------------------ protocol

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineOp {
    Start,
    Stop,
    Restart,
}

#[derive(Debug)]
#[allow(dead_code)] // machine and Kubernetes fields are for features still to come
pub enum Cmd {
    Refresh,
    Select(String),
    Start(String),
    Stop(String),
    Restart(String),
    Pause(String),
    Unpause(String),
    Remove(String),
    Run { image: String, name: String, host: String, port: String },
    Pull(String),
    RemoveImage(String),
    PruneImages,
    CreateVolume(String),
    RemoveVolume(String),
    CreateNetwork(String),
    RemoveNetwork(String),
    Engine(EngineOp),
    MachineStart(String),
    MachineStop(String),
    MachineCreate { distro: String, name: String },
    MachineDelete(String),
    Kube(bool),
    KubeReset,
    Config(String, String),
    ApplyResources { cpus: u32, mem_mib: u32 },
    OpenTerminal(String),
    /// The Builds view opened or closed (it refreshes faster while open).
    BuildsOpen(bool),
    /// The same for the Kubernetes view.
    KubeOpen(bool),
    KubeDelete { kind: String, ns: String, name: String },
    KubeScale { kind: String, ns: String, name: String, replicas: u32 },
    BuildInfo(String),
    /// Stream a container's logs to the log page, from `tail` lines back.
    OpenLogs { id: String, tail: Option<u32> },
    OpenBuildLog(String),
    CloseLogs,
}

#[derive(Clone, Debug, Default)]
pub struct Info {
    /// The guest OS, as Docker reports it.
    pub runtime: String,
    pub version: String,
    pub cpus: u32,
    pub mem_gb: f64,
    pub arch: String,
    pub endpoint: String,
    pub disk_gb: f64,
    /// The engine was started from an older build and should be restarted.
    pub stale: bool,
}

#[derive(Clone, Debug)]
pub struct BoxSnap {
    pub id: String,
    pub hex: String,
    pub image: String,
    pub port: String,
    pub ports_text: String,
    pub grp: String,
    pub net: String,
    pub st: St,
    pub tag: String,
    pub bad: bool,
    pub loops: u32,
    pub up: String,
}

#[derive(Clone, Debug)]
pub struct MachSnap {
    pub id: String,
    pub distro: String,
    pub st: St,
    pub arch: String,
}

#[derive(Clone, Debug)]
pub struct PodSnap {
    pub id: String,
    pub ready: String,
    pub phase: String,
    pub running: bool,
    /// Its Docker container, for logs.
    pub container: String,
}

#[allow(dead_code)] // `Machines` is for Linux machines, still to come
pub enum Ev {
    Up(Info),
    /// Docker isn't reachable. `status` is the engine's own state when its
    /// process is running (booting, installing, stopping, error).
    Down { installed: bool, status: Option<engine::control::Status> },
    Containers(Vec<BoxSnap>),
    Stats { id: String, cpu: f64, mem_mb: f64, limit_mb: f64 },
    Inspect { id: String, why: String, restart: String, hostname: String, loops: u32 },
    Logs { id: String, lines: Vec<(String, String, u8)>, reset: bool },
    Assets { imgs: Vec<Asset>, vols: Vec<Asset>, nets: Vec<Net> },
    /// `None` while Linux machines aren't available.
    Machines(Option<Vec<MachSnap>>),
    Kube { enabled: bool, reachable: bool, pods: Vec<PodSnap> },
    /// What the cluster's API reports: workloads, services, storage,
    /// config and metrics, or why it can't be read.
    KubeData(Result<engine::kube::Snapshot, String>),
    Config(HashMap<String, String>),
    Pull { image: String, layers: Vec<(String, f64)>, done: bool, err: Option<String> },
    /// Total disk used by images, containers and volumes, in GB.
    Disk(f64),
    /// An action finished. `key` says which; `msg` is a toast or an error.
    Done { key: String, ok: bool, msg: String },
    /// The build history, or why it can't be read.
    Builds(Result<Vec<Build>, String>),
    BuildInfo { id: String, info: BuildInfo },
    /// Lines for the log page showing `page`; `reset` replaces what it has.
    PageLogs { page: Page, lines: Vec<(String, String, u8)>, reset: bool },
}

/// The UI's end of the connection.
pub struct Handle {
    tx: tmpsc::UnboundedSender<Cmd>,
    rx: mpsc::Receiver<Ev>,
}

impl Handle {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn drain(&self) -> Vec<Ev> {
        self.rx.try_iter().collect()
    }
}

pub fn spawn(ctx: egui::Context) -> Handle {
    let (tx, cmd_rx) = tmpsc::unbounded_channel();
    let (ev_tx, rx) = mpsc::channel();
    let out = Out { tx: ev_tx, ctx, cmd: tx.clone() };
    std::thread::Builder::new()
        .name("patina-docker".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime");
            rt.block_on(worker(cmd_rx, out));
        })
        .expect("docker thread");
    Handle { tx, rx }
}

#[derive(Clone)]
struct Out {
    tx: mpsc::Sender<Ev>,
    ctx: egui::Context,
    cmd: tmpsc::UnboundedSender<Cmd>,
}

impl Out {
    fn send(&self, ev: Ev) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
    fn refresh(&self) {
        let _ = self.cmd.send(Cmd::Refresh);
    }
    fn done(&self, key: impl Into<String>, r: Result<String, String>) {
        let (ok, msg) = match r {
            Ok(m) => (true, m),
            Err(e) => (false, e),
        };
        self.send(Ev::Done { key: key.into(), ok, msg });
        self.refresh();
    }
}

fn endpoint() -> String {
    format!("unix://{}", paths::docker_sock().display())
}

fn connect() -> Option<Docker> {
    Docker::connect_with_unix(&paths::docker_sock().to_string_lossy(), 30, API_DEFAULT_VERSION).ok()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

// ------------------------------------------------------------------- helpers

/// Capitalize and end with a period, the way the design writes messages.
fn sentence(s: &str) -> String {
    let s = s.trim().trim_start_matches("Error: ").trim_start_matches("error: ");
    let mut c = s.chars();
    let mut out = match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => return "Something went wrong.".into(),
    };
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

fn nice(e: &DockerError) -> String {
    match e {
        DockerError::DockerResponseServerError { message, .. } => sentence(message),
        DockerError::SocketNotFoundError(_) => "Docker is not running.".into(),
        other => sentence(&other.to_string()),
    }
}

pub fn ago(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        return "just now".into();
    }
    let (n, unit) = if s < 3600 {
        (s / 60, "minute")
    } else if s < 86_400 {
        (s / 3600, "hour")
    } else if s < 86_400 * 14 {
        (s / 86_400, "day")
    } else if s < 86_400 * 60 {
        (s / (86_400 * 7), "week")
    } else if s < 86_400 * 365 {
        (s / (86_400 * 30), "month")
    } else {
        (s / (86_400 * 365), "year")
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out
}

fn classify(m: &str) -> u8 {
    let l = m.to_lowercase();
    if ["error", "fatal", "panic", "exception", "failed", "refused"].iter().any(|w| l.contains(w)) {
        3
    } else if l.contains("warn") {
        2
    } else if ["started", "ready", "listening", "success", "complete", "healthy"].iter().any(|w| l.contains(w)) {
        1
    } else {
        0
    }
}

/// Split a chunk of `docker logs --timestamps` output into (time, message, kind).
fn log_lines(bytes: &[u8]) -> Vec<(String, String, u8)> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (ts, msg) = l.split_once(' ').unwrap_or(("", l));
            let t = chrono::DateTime::parse_from_rfc3339(ts)
                .map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
                .unwrap_or_else(|_| chrono::Local::now().format("%H:%M:%S").to_string());
            let msg = strip_ansi(if ts.contains('T') { msg } else { l });
            let k = classify(&msg);
            (t, msg, k)
        })
        .collect()
}

/// Follow a container's logs for the log page, in batches so a long
/// history arrives as a few big updates rather than thousands of small ones.
async fn stream_page(d: Docker, out: Out, page: Page, id: String, tail: String, reset: bool) {
    if reset {
        out.send(Ev::PageLogs { page: page.clone(), lines: Vec::new(), reset: true });
    }
    let opts = LogsOptionsBuilder::new().stdout(true).stderr(true).timestamps(true).follow(true).tail(&tail).build();
    let mut s = d.logs(&id, Some(opts));
    let mut batch = Vec::new();
    loop {
        let ended = match tokio::time::timeout(Duration::from_millis(50), s.next()).await {
            Ok(Some(Ok(chunk))) => {
                batch.extend(log_lines(&chunk.into_bytes()));
                if batch.len() < 5000 {
                    continue;
                }
                false
            }
            Ok(Some(Err(_)) | None) => true,
            Err(_) => false, // quiet for a moment: send what we have
        };
        if !batch.is_empty() {
            out.send(Ev::PageLogs { page: page.clone(), lines: std::mem::take(&mut batch), reset: false });
        }
        if ended {
            break;
        }
    }
}

/// A build's log, read again every two seconds while the build runs.
async fn build_log(out: Out, page: Page, id: String) {
    loop {
        let lines = match buildx(&["history", "logs", &id]).await {
            Ok(text) => text.lines().map(|l| (String::new(), strip_ansi(l), classify(l))).collect(),
            Err(e) => vec![(String::new(), e, 3)],
        };
        out.send(Ev::PageLogs { page: page.clone(), lines, reset: true });
        let running = buildx(&["history", "inspect", &id, "--format", "json"])
            .await
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .is_some_and(|v| v["Status"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("running")));
        if !running {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

const NO_BUILDX: &str = "Patina reads build history through docker buildx, which it couldn't find. Patina installs it when it starts; check your connection, reopen Patina, then refresh.";

/// Run `docker buildx …` against Patina's engine with its current builder.
async fn buildx(args: &[&str]) -> Result<String, String> {
    let cmd = tokio::process::Command::new(paths::docker_cli())
        .args(["--context", "patina", "buildx"])
        .args(args)
        .env_remove("DOCKER_CONTEXT")
        .env_remove("DOCKER_HOST")
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(20), cmd).await.map_err(|_| "docker buildx didn't answer.".to_string())?.map_err(|e| e.to_string())?;
    if out.status.success() {
        // `history logs` writes the log to stderr, like build progress.
        let text = if out.stdout.is_empty() { &out.stderr } else { &out.stdout };
        return Ok(String::from_utf8_lossy(text).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("is not a docker command") || err.contains("unknown command") {
        Err(NO_BUILDX.into())
    } else {
        Err(sentence(err.lines().find(|l| !l.trim().is_empty()).unwrap_or("docker buildx failed")))
    }
}

fn unix(ts: &serde_json::Value) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(ts.as_str()?).ok().map(|t| t.timestamp_millis() as f64 / 1000.0)
}

/// `docker buildx history ls --format json`: one JSON object per line.
fn parse_builds(text: &str) -> Vec<Build> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            let created = unix(&v["created_at"]).unwrap_or(0.0);
            let st = match v["status"].as_str().unwrap_or("").to_lowercase().as_str() {
                "running" => BuildSt::Running,
                "error" | "failed" => BuildSt::Failed,
                "canceled" | "cancelled" => BuildSt::Canceled,
                _ => BuildSt::Done,
            };
            let n = |k: &str| v[k].as_u64().unwrap_or(0) as u32;
            Build {
                id: v["ref"].as_str().unwrap_or("").rsplit('/').next().unwrap_or("").to_string(),
                name: v["name"].as_str().unwrap_or("").to_string(),
                st,
                created: created as i64,
                secs: unix(&v["completed_at"]).map_or(0.0, |c| (c - created).max(0.0)),
                steps: n("total_steps"),
                done_steps: n("completed_steps"),
                cached: n("cached_steps"),
                err: v["error"].as_str().map(str::to_string).unwrap_or_default(),
            }
        })
        .filter(|b| !b.id.is_empty())
        .collect()
}

/// `docker buildx history inspect --format json`.
fn parse_build_info(json: &str) -> BuildInfo {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return BuildInfo::default() };
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let list = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    let args = v["BuildArgs"]
        .as_array()
        .map(|a| a.iter().map(|x| format!("{}={}", x["Name"].as_str().unwrap_or(""), x["Value"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("  "))
        .unwrap_or_default();
    let err = v["Error"]["Message"].as_str().or_else(|| v["Error"].as_str()).unwrap_or("").to_string();
    BuildInfo { dockerfile: s("Dockerfile"), context: s("Context"), platform: list("Platform"), repo: s("VCSRepository"), revision: s("VCSRevision"), args, err }
}

fn snap(c: &ContainerSummary) -> BoxSnap {
    let id = c.names.as_ref().and_then(|n| n.first()).map(|n| n.trim_start_matches('/').to_string()).unwrap_or_default();
    let hex: String = c.id.clone().unwrap_or_default().chars().take(12).collect();
    let status = c.status.clone().unwrap_or_default();
    let labels = c.labels.clone().unwrap_or_default();
    let grp = labels.get("com.docker.compose.project").cloned().unwrap_or_else(|| "standalone".into());
    let exit_code = status.split_once('(').and_then(|(_, r)| r.split_once(')')).and_then(|(n, _)| n.trim().parse::<i64>().ok()).unwrap_or(0);
    let unhealthy = status.contains("(unhealthy)");
    use ContainerSummaryStateEnum as S;
    let (st, tag, loops) = match c.state {
        Some(S::RUNNING) => (St::Running, if unhealthy { "unhealthy".to_string() } else { String::new() }, 0),
        Some(S::PAUSED) => (St::Paused, String::new(), 0),
        Some(S::RESTARTING) => (St::Starting, "restarting".to_string(), 1),
        Some(S::EXITED) if exit_code != 0 => (St::Error, format!("exited {exit_code}"), 0),
        Some(S::DEAD) => (St::Error, "dead".to_string(), 0),
        Some(S::REMOVING) | Some(S::STOPPING) => (St::Stopping, String::new(), 0),
        _ => (St::Stopped, String::new(), 0),
    };
    let up = status
        .strip_prefix("Up ")
        .map(|s| {
            let s = s.split(" (").next().unwrap_or(s);
            let mut ch = s.chars();
            ch.next().map(|f| f.to_lowercase().collect::<String>() + ch.as_str()).unwrap_or_default()
        })
        .unwrap_or_default();
    let mut ports: Vec<(u16, u16, String)> = c
        .ports
        .iter()
        .flatten()
        .filter_map(|p| p.public_port.map(|pub_| (pub_, p.private_port, p.typ.map(|t| t.to_string()).unwrap_or_else(|| "tcp".into()))))
        .collect();
    ports.sort();
    ports.dedup();
    let port = ports.first().map(|p| p.0.to_string()).unwrap_or_default();
    let ports_text = if ports.is_empty() {
        "none published".to_string()
    } else {
        ports.iter().map(|(h, p, t)| format!("{h} → {p}/{t}")).collect::<Vec<_>>().join(", ")
    };
    let mut nets: Vec<String> = c.network_settings.as_ref().and_then(|n| n.networks.as_ref()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    nets.sort();
    BoxSnap {
        id,
        hex,
        image: c.image.clone().unwrap_or_default(),
        port,
        ports_text,
        grp,
        net: nets.first().cloned().unwrap_or_else(|| "none".into()),
        st,
        tag,
        bad: unhealthy,
        loops,
        up,
    }
}

#[derive(Clone)]
struct Seen {
    name: String,
    image_id: String,
    running: bool,
    volumes: Vec<String>,
    networks: Vec<String>,
}

fn seen(c: &ContainerSummary) -> Seen {
    Seen {
        name: c.names.as_ref().and_then(|n| n.first()).map(|n| n.trim_start_matches('/').to_string()).unwrap_or_default(),
        image_id: c.image_id.clone().unwrap_or_default(),
        running: c.state == Some(ContainerSummaryStateEnum::RUNNING),
        volumes: c.mounts.iter().flatten().filter_map(|m| m.name.clone()).filter(|n| !n.is_empty()).collect(),
        networks: c.network_settings.as_ref().and_then(|n| n.networks.as_ref()).map(|m| m.keys().cloned().collect()).unwrap_or_default(),
    }
}

/// The name a user would recognize for a container using something, preferring running ones.
fn user_of<'a>(seen: &'a [Seen], hit: impl Fn(&Seen) -> bool) -> Option<&'a str> {
    let mut hits: Vec<&Seen> = seen.iter().filter(|s| hit(s)).collect();
    hits.sort_by_key(|s| !s.running);
    hits.first().map(|s| s.name.as_str())
}

/// Docker's image reference split into (image, tag), defaulting to `latest`.
fn split_ref(image: &str) -> (String, Option<String>) {
    if image.contains('@') {
        return (image.to_string(), None);
    }
    let slash = image.rfind('/').map_or(0, |i| i + 1);
    match image[slash..].rfind(':') {
        Some(i) => (image[..slash + i].to_string(), Some(image[slash + i + 1..].to_string())),
        None => (image.to_string(), Some("latest".to_string())),
    }
}

// -------------------------------------------------------------------- worker

struct W {
    out: Out,
    docker: Option<Docker>,
    up: bool,
    tick: u64,
    seen: Vec<Seen>,
    boxes: Vec<BoxSnap>,
    stats: HashMap<String, JoinHandle<()>>,
    sel: String,
    logs: Option<(String, JoinHandle<()>)>,
    vol_sizes: Arc<std::sync::Mutex<HashMap<String, f64>>>,
    assets_busy: Arc<AtomicBool>,
    df_busy: Arc<AtomicBool>,
    builds_busy: Arc<AtomicBool>,
    builds_open: bool,
    kube_busy: Arc<AtomicBool>,
    kube_open: bool,
    /// What the log page shows (and how far back), and its stream. The
    /// stream is restarted when it's missing, e.g. after the engine restarts.
    page: Option<(Page, Option<u32>)>,
    page_task: Option<JoinHandle<()>>,
    last_down: Option<Instant>,
}

async fn worker(mut rx: tmpsc::UnboundedReceiver<Cmd>, out: Out) {
    let mut w = W {
        out,
        docker: None,
        up: false,
        tick: 0,
        seen: Vec::new(),
        boxes: Vec::new(),
        stats: HashMap::new(),
        sel: String::new(),
        logs: None,
        vol_sizes: Arc::new(std::sync::Mutex::new(HashMap::new())),
        assets_busy: Arc::new(AtomicBool::new(false)),
        df_busy: Arc::new(AtomicBool::new(false)),
        builds_busy: Arc::new(AtomicBool::new(false)),
        builds_open: false,
        kube_busy: Arc::new(AtomicBool::new(false)),
        kube_open: false,
        page: None,
        page_task: None,
        last_down: None,
    };
    // Existing installs from before PATH integration or buildx get them now.
    let _ = blocking(|| {
        if engine::setup::installed() && config::load().shell_path && !integrate::shell_installed() {
            let _ = integrate::install_shell();
        }
        if engine::setup::installed() {
            let _ = engine::setup::install_plugins(&|_, _| {});
        }
    })
    .await;
    w.refresh(true).await;
    let mut iv = tokio::time::interval(Duration::from_millis(1000));
    iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(Cmd::Refresh) => w.refresh(true).await,
                Some(c) => w.handle(c).await,
                None => break,
            },
            _ = iv.tick() => w.refresh(false).await,
        }
    }
}

impl W {
    fn stop_streams(&mut self) {
        for (_, h) in self.stats.drain() {
            h.abort();
        }
        if let Some((_, h)) = self.logs.take() {
            h.abort();
        }
        if let Some(h) = self.page_task.take() {
            h.abort();
        }
    }

    async fn refresh(&mut self, forced: bool) {
        self.tick += 1;
        if !self.up {
            if !forced && self.tick % 2 == 0 {
                return;
            }
            // Only talk to Docker once the engine says it's running.
            let status = blocking(control::status).await.flatten();
            match status {
                Some(s) if s.state == "running" => {}
                other => {
                    self.docker = None;
                    self.down(other);
                    return;
                }
            }
        }
        if self.docker.is_none() {
            self.docker = connect();
        }
        let Some(d) = self.docker.clone() else {
            self.down(None);
            return;
        };
        let list = match d.list_containers(Some(ListContainersOptionsBuilder::new().all(true).build())).await {
            Ok(l) => l,
            Err(_) => {
                self.docker = None;
                if self.up {
                    self.up = false;
                    self.stop_streams();
                }
                let status = blocking(control::status).await.flatten();
                self.down(status);
                return;
            }
        };
        let first = !self.up;
        self.up = true;
        if first || self.tick % 30 == 0 {
            self.send_info(&d).await;
            self.send_config();
        }
        self.seen = list.iter().map(seen).collect();
        let kube_on = config::load().kubernetes;
        self.send_kube(&list, kube_on);
        if kube_on && (first || forced || self.tick.is_multiple_of(if self.kube_open { 3 } else { 15 })) {
            self.kube_data(&d);
        }
        let mut boxes: Vec<BoxSnap> = list.iter().filter(|c| !engine::kube::hidden(c)).map(snap).collect();
        boxes.sort_by(|a, b| a.id.cmp(&b.id));
        self.boxes = boxes.clone();
        self.out.send(Ev::Containers(boxes));
        self.sync_stats(&d);
        self.sync_logs(&d);
        self.sync_page();
        if first || forced || self.tick.is_multiple_of(if self.builds_open { 2 } else { 15 }) {
            self.builds();
        }
        if first || forced || self.tick % 3 == 0 {
            self.assets(&d);
        }
        if first || self.tick % 20 == 0 {
            self.df(&d);
        }
        if first || forced || self.tick % 2 == 0 {
            self.inspect_sel(&d).await;
        }
    }

    fn down(&mut self, status: Option<control::Status>) {
        let due = status.is_some() || self.last_down.is_none_or(|t| t.elapsed() > Duration::from_millis(1500));
        if due {
            self.last_down = Some(Instant::now());
            self.out.send(Ev::Down { installed: engine::setup::installed(), status });
            self.out.send(Ev::Machines(None));
            self.send_config();
        }
    }

    fn send_config(&self) {
        let c = config::load();
        let mut map = HashMap::new();
        map.insert("cpu".to_string(), c.cpus.to_string());
        map.insert("memory_mib".to_string(), (c.memory_gb * 1024).to_string());
        map.insert("rosetta".to_string(), c.rosetta.to_string());
        map.insert("autostart".to_string(), c.autostart.to_string());
        map.insert("login".to_string(), c.login.to_string());
        map.insert("set_context".to_string(), c.set_context.to_string());
        map.insert("shell_path".to_string(), c.shell_path.to_string());
        map.insert("menubar".to_string(), c.menubar.to_string());
        self.out.send(Ev::Config(map));
    }

    async fn send_info(&self, d: &Docker) {
        let Ok(info) = d.info().await else { return };
        let arch = match info.architecture.as_deref() {
            Some("aarch64") | Some("arm64") => "arm64".to_string(),
            Some("x86_64") => "amd64".to_string(),
            Some(a) => a.to_string(),
            None => String::new(),
        };
        self.out.send(Ev::Up(Info {
            runtime: info.operating_system.unwrap_or_default(),
            version: info.server_version.unwrap_or_default(),
            cpus: info.ncpu.unwrap_or(0) as u32,
            mem_gb: info.mem_total.unwrap_or(0) as f64 / (1u64 << 30) as f64,
            arch,
            endpoint: endpoint(),
            disk_gb: 0.0,
            stale: blocking(control::status).await.flatten().is_some_and(|s| control::stale(&s)),
        }));
    }

    fn sync_stats(&mut self, d: &Docker) {
        let running: HashSet<String> = self.boxes.iter().filter(|b| b.st == St::Running).map(|b| b.id.clone()).collect();
        self.stats.retain(|id, h| {
            let keep = running.contains(id) && !h.is_finished();
            if !keep {
                h.abort();
            }
            keep
        });
        for id in running {
            if self.stats.contains_key(&id) {
                continue;
            }
            let d = d.clone();
            let out = self.out.clone();
            let name = id.clone();
            let h = tokio::spawn(async move {
                let mut s = d.stats(&name, Some(StatsOptionsBuilder::new().stream(true).build()));
                while let Some(Ok(st)) = s.next().await {
                    let (Some(cpu), Some(pre)) = (st.cpu_stats.as_ref(), st.precpu_stats.as_ref()) else { continue };
                    let total = |c: &bollard::models::ContainerCpuStats| c.cpu_usage.as_ref().and_then(|u| u.total_usage).unwrap_or(0) as f64;
                    let cpu_d = total(cpu) - total(pre);
                    let sys_d = cpu.system_cpu_usage.unwrap_or(0) as f64 - pre.system_cpu_usage.unwrap_or(0) as f64;
                    let online = cpu
                        .online_cpus
                        .map(|n| n as f64)
                        .or_else(|| cpu.cpu_usage.as_ref().and_then(|u| u.percpu_usage.as_ref()).map(|v| v.len() as f64))
                        .unwrap_or(1.0);
                    let pct = if sys_d > 0.0 && cpu_d >= 0.0 { cpu_d / sys_d * online * 100.0 } else { 0.0 };
                    let mem = st.memory_stats.as_ref();
                    let usage = mem.and_then(|m| m.usage).unwrap_or(0) as f64;
                    let cache = mem
                        .and_then(|m| m.stats.as_ref())
                        .and_then(|s| s.get("inactive_file").or_else(|| s.get("total_inactive_file")).or_else(|| s.get("cache")).copied())
                        .unwrap_or(0) as f64;
                    let limit = mem.and_then(|m| m.limit).unwrap_or(0) as f64;
                    out.send(Ev::Stats { id: name.clone(), cpu: pct, mem_mb: ((usage - cache).max(0.0)) / 1_048_576.0, limit_mb: limit / 1_048_576.0 });
                }
            });
            self.stats.insert(id, h);
        }
    }

    fn sync_logs(&mut self, d: &Docker) {
        if self.sel.is_empty() {
            return;
        }
        let running = self.boxes.iter().find(|b| b.id == self.sel).is_some_and(|b| matches!(b.st, St::Running | St::Starting | St::Paused));
        if let Some((id, h)) = &self.logs {
            if *id == self.sel && (!h.is_finished() || !running) {
                return;
            }
        }
        let reset = self.logs.as_ref().is_none_or(|(id, _)| *id != self.sel);
        if let Some((_, h)) = self.logs.take() {
            h.abort();
        }
        let d = d.clone();
        let out = self.out.clone();
        let name = self.sel.clone();
        let h = tokio::spawn(async move {
            if reset {
                let mut lines = Vec::new();
                let mut s = d.logs(&name, Some(LogsOptionsBuilder::new().stdout(true).stderr(true).timestamps(true).tail("10").build()));
                while let Some(item) = s.next().await {
                    match item {
                        Ok(chunk) => lines.extend(log_lines(&chunk.into_bytes())),
                        Err(e) => {
                            debug::<()>("logs", &Err(e));
                            break;
                        }
                    }
                }
                let n = lines.len();
                let lines = lines.split_off(n.saturating_sub(10));
                out.send(Ev::Logs { id: name.clone(), lines, reset: true });
            }
            if running {
                let mut s = d.logs(
                    &name,
                    Some(LogsOptionsBuilder::new().stdout(true).stderr(true).timestamps(true).follow(true).tail("0").build()),
                );
                while let Some(Ok(chunk)) = s.next().await {
                    let lines = log_lines(&chunk.into_bytes());
                    if !lines.is_empty() {
                        out.send(Ev::Logs { id: name.clone(), lines, reset: false });
                    }
                }
            }
        });
        self.logs = Some((self.sel.clone(), h));
    }

    /// Start the log page's stream when it has none. A container's stream
    /// also ends when the container stops; it picks up again when it runs,
    /// with only the new lines.
    fn sync_page(&mut self) {
        let Some((page, tail)) = self.page.clone() else { return };
        let out = self.out.clone();
        match (&page, &self.page_task) {
            (Page::BuildLog(id), None) => self.page_task = Some(tokio::spawn(build_log(out, page.clone(), id.clone()))),
            (Page::Logs(id), task) => {
                let Some(d) = self.docker.clone() else { return };
                let running = self.boxes.iter().any(|b| b.id == *id && matches!(b.st, St::Running | St::Starting));
                let from = match task {
                    None => Some((tail.map_or_else(|| "all".to_string(), |n| n.to_string()), true)),
                    Some(h) if h.is_finished() && running => Some(("0".to_string(), false)),
                    Some(_) => None,
                };
                if let Some((tail, reset)) = from {
                    self.page_task = Some(tokio::spawn(stream_page(d, out, page.clone(), id.clone(), tail, reset)));
                }
            }
            _ => {}
        }
    }

    fn open_page(&mut self, page: Option<(Page, Option<u32>)>) {
        if let Some(h) = self.page_task.take() {
            h.abort();
        }
        self.page = page;
        self.sync_page();
    }

    /// The cluster's state and pods, from the pods' containers.
    fn send_kube(&self, list: &[ContainerSummary], enabled: bool) {
        let pods: Vec<PodSnap> = engine::kube::pods(list)
            .into_iter()
            .map(|p| PodSnap { id: p.id, ready: p.ready, phase: p.phase, running: p.ok, container: p.container })
            .collect();
        let reachable = pods.iter().any(|p| p.running && !p.id.contains('/'));
        self.out.send(Ev::Kube { enabled, reachable, pods });
    }

    /// Read the cluster's API off the main loop.
    fn kube_data(&self, d: &Docker) {
        if self.kube_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (d, out, busy) = (d.clone(), self.out.clone(), self.kube_busy.clone());
        tokio::spawn(async move {
            let _ = engine::kube::ensure_creds(&d).await;
            let r = blocking(|| engine::kube::Api::load().and_then(|api| engine::kube::snapshot(&api))).await.unwrap_or_else(|| Err("The cluster check stopped.".into()));
            out.send(Ev::KubeData(r));
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Build history, read through `docker buildx` off the main loop.
    fn builds(&self) {
        if self.builds_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (out, busy) = (self.out.clone(), self.builds_busy.clone());
        tokio::spawn(async move {
            let r = buildx(&["history", "ls", "--format", "json"]).await.map(|text| parse_builds(&text));
            out.send(Ev::Builds(r));
            busy.store(false, Ordering::SeqCst);
        });
    }

    async fn inspect_sel(&self, d: &Docker) {
        if self.sel.is_empty() || !self.boxes.iter().any(|b| b.id == self.sel) {
            return;
        }
        let Ok(x) = d.inspect_container(&self.sel, None).await else { return };
        let state = x.state.clone().unwrap_or_default();
        let code = state.exit_code.unwrap_or(0);
        let restarts = x.restart_count.unwrap_or(0).max(0) as u32;
        let limit_mb = x.host_config.as_ref().and_then(|h| h.memory).unwrap_or(0) / 1_048_576;
        let health = state.health.as_ref();
        let unhealthy = health.and_then(|h| h.status).map(|s| s.to_string() == "unhealthy").unwrap_or(false);
        let why = if state.oom_killed.unwrap_or(false) {
            if limit_mb > 0 {
                format!("Killed by the kernel: out of memory. The limit is {limit_mb} MB.")
            } else {
                "Killed by the kernel: out of memory.".to_string()
            }
        } else if state.restarting.unwrap_or(false) {
            format!("Exited with code {code} and is being restarted. Attempt {restarts}.")
        } else if !state.running.unwrap_or(false) && code != 0 {
            match state.error.as_deref().filter(|e| !e.is_empty()) {
                Some(e) => sentence(e),
                None => format!("Exited with code {code}."),
            }
        } else if unhealthy {
            let streak = health.and_then(|h| h.failing_streak).unwrap_or(0);
            let last = health
                .and_then(|h| h.log.as_ref())
                .and_then(|l| l.last())
                .and_then(|r| r.output.clone())
                .map(|o| o.lines().next().unwrap_or("").trim().to_string())
                .unwrap_or_default();
            if last.is_empty() {
                format!("The health check failed {streak} times.")
            } else {
                format!("The health check failed {streak} times: {last}")
            }
        } else {
            String::new()
        };
        let restart = x
            .host_config
            .as_ref()
            .and_then(|h| h.restart_policy.as_ref())
            .and_then(|r| r.name)
            .map(|n| n.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "no".into());
        let hostname = x.config.as_ref().and_then(|c| c.hostname.clone()).unwrap_or_default();
        self.out.send(Ev::Inspect { id: self.sel.clone(), why, restart, hostname, loops: if state.restarting.unwrap_or(false) { restarts.max(1) } else { 0 } });
    }

    /// Disk usage is slow to compute (it sizes every volume), so it runs on its own.
    fn df(&self, d: &Docker) {
        if self.df_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (d, out, sizes, busy) = (d.clone(), self.out.clone(), self.vol_sizes.clone(), self.df_busy.clone());
        tokio::spawn(async move {
            if let Ok(df) = d.df(None).await {
                let sum = |x: Option<i64>| x.unwrap_or(0).max(0) as f64;
                let total = (sum(df.image_usage.as_ref().and_then(|u| u.total_size))
                    + sum(df.container_usage.as_ref().and_then(|u| u.total_size))
                    + sum(df.volume_usage.as_ref().and_then(|u| u.total_size)))
                    / 1e9;
                if let Ok(mut map) = sizes.lock() {
                    for v in df.volume_usage.and_then(|u| u.items).unwrap_or_default() {
                        if let (Some(n), Some(sz)) = (v["Name"].as_str(), v["UsageData"]["Size"].as_i64()) {
                            if sz >= 0 {
                                map.insert(n.to_string(), sz as f64 / 1e6);
                            }
                        }
                    }
                }
                out.send(Ev::Disk(total));
                out.refresh();
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Images, volumes and networks, listed off the main loop.
    fn assets(&self, d: &Docker) {
        if self.assets_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (d, out, sizes, busy) = (d.clone(), self.out.clone(), self.vol_sizes.clone(), self.assets_busy.clone());
        let seen = self.seen.clone();
        tokio::spawn(async move {
            let ev = list_assets(&d, &seen, &sizes).await;
            out.send(ev);
            busy.store(false, Ordering::SeqCst);
        });
    }

    fn need_docker(&self, key: &str) -> Option<Docker> {
        if self.docker.is_none() {
            self.out.done(key, Err("Docker is not running.".into()));
        }
        self.docker.clone()
    }

    async fn handle(&mut self, cmd: Cmd) {
        let out = self.out.clone();
        match cmd {
            Cmd::Refresh => {}
            Cmd::KubeDelete { kind, ns, name } => {
                tokio::spawn(async move {
                    let what = format!("{} {name}", crate::sim::kind_word(&kind));
                    let r = blocking(move || engine::kube::Api::load()?.delete(&kind, &ns, &name)).await.unwrap_or_else(|| Err("The delete stopped.".into()));
                    out.done("kube-act", r.map(|_| format!("Deleted {what}")));
                });
            }
            Cmd::KubeScale { kind, ns, name, replicas } => {
                tokio::spawn(async move {
                    let msg = format!("Scaled {name} to {replicas}");
                    let r = blocking(move || engine::kube::Api::load()?.scale(&kind, &ns, &name, replicas)).await.unwrap_or_else(|| Err("Scaling stopped.".into()));
                    out.done("kube-act", r.map(|_| msg));
                });
            }
            Cmd::KubeOpen(open) => {
                self.kube_open = open;
                if open && config::load().kubernetes {
                    if let Some(d) = self.docker.clone() {
                        self.kube_data(&d);
                    }
                }
            }
            Cmd::BuildsOpen(open) => {
                self.builds_open = open;
                if open {
                    self.builds();
                }
            }
            Cmd::BuildInfo(id) => {
                tokio::spawn(async move {
                    if let Ok(json) = buildx(&["history", "inspect", &id, "--format", "json"]).await {
                        out.send(Ev::BuildInfo { id, info: parse_build_info(&json) });
                    }
                });
            }
            Cmd::OpenLogs { id, tail } => self.open_page(Some((Page::Logs(id), tail))),
            Cmd::OpenBuildLog(id) => self.open_page(Some((Page::BuildLog(id), None))),
            Cmd::CloseLogs => self.open_page(None),
            Cmd::Select(id) => {
                self.sel = id;
                if let Some(d) = self.docker.clone() {
                    self.sync_logs(&d);
                    self.inspect_sel(&d).await;
                }
            }
            Cmd::Start(id) | Cmd::Stop(id) | Cmd::Restart(id) | Cmd::Pause(id) | Cmd::Unpause(id) | Cmd::Remove(id) if self.docker.is_none() => {
                out.done(format!("box:{id}"), Err("Docker is not running.".into()));
            }
            Cmd::Start(id) => {
                let d = self.docker.clone().unwrap();
                tokio::spawn(async move {
                    let r = d.start_container(&id, None).await.map(|_| String::new()).map_err(|e| nice(&e));
                    out.done(format!("box:{id}"), r);
                });
            }
            Cmd::Stop(id) => {
                let d = self.docker.clone().unwrap();
                tokio::spawn(async move {
                    let r = d.stop_container(&id, None).await.map(|_| String::new()).map_err(|e| nice(&e));
                    out.done(format!("box:{id}"), r);
                });
            }
            Cmd::Restart(id) => {
                let d = self.docker.clone().unwrap();
                tokio::spawn(async move {
                    let r = d.restart_container(&id, None).await.map(|_| String::new()).map_err(|e| nice(&e));
                    out.done(format!("box:{id}"), r);
                });
            }
            Cmd::Pause(id) => {
                let d = self.docker.clone().unwrap();
                tokio::spawn(async move {
                    let r = d.pause_container(&id).await.map(|_| String::new()).map_err(|e| nice(&e));
                    out.done(format!("box:{id}"), r);
                });
            }
            Cmd::Unpause(id) => {
                let d = self.docker.clone().unwrap();
                tokio::spawn(async move {
                    let r = d.unpause_container(&id).await.map(|_| String::new()).map_err(|e| nice(&e));
                    out.done(format!("box:{id}"), r);
                });
            }
            Cmd::Remove(id) => {
                let d = self.docker.clone().unwrap();
                tokio::spawn(async move {
                    let r = d
                        .remove_container(&id, Some(RemoveContainerOptionsBuilder::new().force(true).build()))
                        .await
                        .map(|_| format!("Deleted {id}"))
                        .map_err(|e| nice(&e));
                    out.done(format!("rm:{id}"), r);
                });
            }
            Cmd::Run { image, name, host, port } => {
                let Some(d) = self.need_docker("run") else { return };
                tokio::spawn(async move {
                    let r = run_container(&d, &image, &name, &host, &port).await;
                    out.done("run", r.map(|_| name));
                });
            }
            Cmd::Pull(image) => {
                let Some(d) = self.need_docker("pull") else { return };
                tokio::spawn(async move { pull(&d, &image, &out).await });
            }
            Cmd::RemoveImage(n) => {
                let Some(d) = self.need_docker(&format!("asset:{n}")) else { return };
                tokio::spawn(async move {
                    let r = d
                        .remove_image(&n, Some(RemoveImageOptionsBuilder::new().build()), None)
                        .await
                        .map(|_| format!("Deleted {n}"))
                        .map_err(|e| nice(&e));
                    out.done(format!("asset:{n}"), r);
                });
            }
            Cmd::PruneImages => {
                let Some(d) = self.need_docker("prune") else { return };
                tokio::spawn(async move {
                    let mut f = HashMap::new();
                    f.insert("dangling", vec!["false"]);
                    let r = d
                        .prune_images(Some(PruneImagesOptionsBuilder::new().filters(&f).build()))
                        .await
                        .map(|p| format!("Freed {}", crate::sim::fmt_size(p.space_reclaimed.unwrap_or(0) as f64 / 1e6)))
                        .map_err(|e| nice(&e));
                    out.done("prune", r);
                });
            }
            Cmd::CreateVolume(n) => {
                let Some(d) = self.need_docker(&format!("create:{n}")) else { return };
                tokio::spawn(async move {
                    let req = VolumeCreateRequest { name: Some(n.clone()), ..Default::default() };
                    let r = d.create_volume(req).await.map(|_| format!("Created {n}")).map_err(|e| nice(&e));
                    out.done(format!("create:{n}"), r);
                });
            }
            Cmd::RemoveVolume(n) => {
                let Some(d) = self.need_docker(&format!("asset:{n}")) else { return };
                tokio::spawn(async move {
                    let r = d.remove_volume(&n, None::<RemoveVolumeOptions>).await.map(|_| format!("Deleted {n}")).map_err(|e| nice(&e));
                    out.done(format!("asset:{n}"), r);
                });
            }
            Cmd::CreateNetwork(n) => {
                let Some(d) = self.need_docker(&format!("create:{n}")) else { return };
                tokio::spawn(async move {
                    let req = NetworkCreateRequest { name: n.clone(), driver: Some("bridge".into()), ..Default::default() };
                    let r = d.create_network(req).await.map(|_| format!("Created {n}")).map_err(|e| nice(&e));
                    out.done(format!("create:{n}"), r);
                });
            }
            Cmd::RemoveNetwork(n) => {
                let Some(d) = self.need_docker(&format!("asset:{n}")) else { return };
                tokio::spawn(async move {
                    let r = d.remove_network(&n).await.map(|_| format!("Deleted {n}")).map_err(|e| nice(&e));
                    out.done(format!("asset:{n}"), r);
                });
            }
            Cmd::Engine(op) => {
                if op != EngineOp::Start {
                    self.docker = None;
                    self.up = false;
                    self.stop_streams();
                }
                tokio::spawn(async move {
                    let r = engine_op(op).await;
                    out.done("engine", r.map(|_| String::new()));
                });
            }
            Cmd::ApplyResources { cpus, mem_mib } => {
                self.docker = None;
                self.up = false;
                self.stop_streams();
                tokio::spawn(async move {
                    let mut c = config::load();
                    c.cpus = cpus;
                    c.memory_gb = (mem_mib / 1024).max(1);
                    let running = blocking(control::status).await.flatten().is_some();
                    let r = match config::save(&c) {
                        Ok(()) if running => engine_op(EngineOp::Restart).await,
                        Ok(()) => Ok(()),
                        Err(e) => Err(e),
                    };
                    out.done("engine", r.map(|_| String::new()));
                });
            }
            Cmd::MachineStart(n) | Cmd::MachineStop(n) | Cmd::MachineDelete(n) => {
                out.done(format!("machine:{n}"), Err("Linux machines aren't available yet.".into()));
            }
            Cmd::MachineCreate { name, .. } => {
                out.done(format!("mk:{name}"), Err("Linux machines aren't available yet.".into()));
            }
            Cmd::Kube(on) => {
                let Some(d) = self.need_docker("kube") else { return };
                tokio::spawn(async move {
                    let r = if on { engine::kube::up(&d).await } else { engine::kube::down(&d, false).await.map(|_| String::new()) };
                    out.done("kube", r);
                });
            }
            Cmd::KubeReset => {
                let Some(d) = self.need_docker("kube") else { return };
                tokio::spawn(async move {
                    let r = match engine::kube::down(&d, true).await {
                        Ok(()) => engine::kube::up(&d).await.map(|_| "The cluster was reset. kubectl uses the patina context.".to_string()),
                        Err(e) => Err(e),
                    };
                    out.done("kube", r);
                });
            }
            Cmd::Config(k, v) => {
                let on = v == "true";
                let mut c = config::load();
                match k.as_str() {
                    "rosetta" => c.rosetta = on,
                    "autostart" => c.autostart = on,
                    "login" => c.login = on,
                    "set_context" => c.set_context = on,
                    "shell_path" => c.shell_path = on,
                    "menubar" => c.menubar = on,
                    _ => {}
                }
                let saved = config::save(&c);
                let key = k.clone();
                // The docker context and shell profiles change right away.
                let applied = blocking(move || match key.as_str() {
                    "set_context" if on => integrate::use_context().map(|_| "The docker command now uses Patina".to_string()),
                    "set_context" => integrate::release_context().map(|_| "The docker command uses the default context again".to_string()),
                    "shell_path" if on => integrate::install_shell().map(|_| "Open a new terminal to use Patina's docker".to_string()),
                    "shell_path" => integrate::remove_shell().map(|_| "Removed Patina from your shell profile".to_string()),
                    _ => Ok(String::new()),
                })
                .await
                .unwrap_or_else(|| Ok(String::new()));
                self.send_config();
                out.done(format!("cfg:{k}"), saved.and(applied));
            }
            Cmd::OpenTerminal(id) => {
                if !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
                    return;
                }
                let docker = paths::docker_cli();
                let host = endpoint();
                let script = format!(
                    "tell application \"Terminal\"\nactivate\ndo script \"'{}' --host '{}' exec -it {id} sh -c 'command -v bash >/dev/null && exec bash || exec sh'\"\nend tell",
                    docker.display(),
                    host
                );
                tokio::spawn(async move {
                    let r = tokio::process::Command::new("/usr/bin/osascript").args(["-e", &script]).output().await;
                    if !r.is_ok_and(|o| o.status.success()) {
                        out.done("terminal", Err("Couldn't open Terminal.".into()));
                    }
                });
            }
        }
    }

}

async fn list_assets(d: &Docker, seen: &[Seen], sizes: &std::sync::Mutex<HashMap<String, f64>>) -> Ev {
    let now = now_unix();
    let mut imgs = Vec::new();
    let r = d.list_images(Some(ListImagesOptionsBuilder::new().all(false).build())).await;
    debug("images", &r);
    if let Ok(list) = r {
        let mut list = list;
        list.sort_by_key(|i| -i.created);
        for i in list {
            let name = i.repo_tags.iter().find(|t| *t != "<none>:<none>").cloned().unwrap_or_else(|| {
                let short: String = i.id.trim_start_matches("sha256:").chars().take(12).collect();
                format!("<none>@{short}")
            });
            let used_by = user_of(&seen, |s| s.image_id == i.id).unwrap_or("").to_string();
            imgs.push(Asset { n: name, mb: i.size.max(0) as f64 / 1e6, used_by, age: ago(now - i.created), fresh: false, gone: false, born: -10.0 });
        }
    }
    let mut vols = Vec::new();
    let r = d.list_volumes(None::<ListVolumesOptions>).await;
    debug("volumes", &r);
    if let Ok(v) = r {
        let mut list = v.volumes.unwrap_or_default();
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        for v in list {
            let created = v.created_at.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.timestamp());
            let used_by = user_of(&seen, |s| s.volumes.contains(&v.name)).unwrap_or("").to_string();
            vols.push(Asset {
                mb: sizes.lock().ok().and_then(|m| m.get(&v.name).copied()).unwrap_or(0.0),
                n: v.name,
                used_by,
                age: created.map(|c| ago(now - c)).unwrap_or_default(),
                fresh: false,
                gone: false,
                born: -10.0,
            });
        }
    }
    let mut nets = Vec::new();
    let r = d.list_networks(None).await;
    debug("networks", &r);
    if let Ok(list) = r {
        for n in list {
            let name = n.name.clone().unwrap_or_default();
            let subnet = n.ipam.as_ref().and_then(|i| i.config.as_ref()).and_then(|c| c.iter().find_map(|c| c.subnet.clone())).unwrap_or_default();
            let grp = n.labels.as_ref().and_then(|l| l.get("com.docker.compose.project").cloned()).unwrap_or_default();
            let attached = seen.iter().filter(|s| s.networks.contains(&name)).count();
            nets.push(Net {
                sys: matches!(name.as_str(), "bridge" | "host" | "none"),
                n: name,
                driver: n.driver.clone().unwrap_or_default(),
                subnet,
                grp,
                fresh: false,
                gone: false,
                born: -10.0,
                attached: Some(attached),
            });
        }
        nets.sort_by(|a, b| b.sys.cmp(&a.sys).then(a.n.cmp(&b.n)));
    }
    Ev::Assets { imgs, vols, nets }
}

/// Start, stop or restart Patina's engine, waiting until it's done.
async fn engine_op(op: EngineOp) -> Result<(), String> {
    if matches!(op, EngineOp::Stop | EngineOp::Restart) {
        if blocking(control::stop).await.unwrap_or(false) {
            for _ in 0..90 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if blocking(control::status).await.flatten().is_none() {
                    break;
                }
            }
        }
        if op == EngineOp::Stop {
            return Ok(());
        }
    }
    blocking(control::start).await.unwrap_or_else(|| Err("The engine didn't start.".into()))
}

async fn run_container(d: &Docker, image: &str, name: &str, host: &str, port: &str) -> Result<(), String> {
    if d.inspect_image(image).await.is_err() {
        let (from, tag) = split_ref(image);
        let mut opts = CreateImageOptionsBuilder::new().from_image(&from);
        if let Some(t) = &tag {
            opts = opts.tag(t);
        }
        let mut s = d.create_image(Some(opts.build()), None, None);
        while let Some(item) = s.next().await {
            match item {
                Ok(info) => {
                    if let Some(e) = info.error_detail.and_then(|e| e.message) {
                        return Err(sentence(&e));
                    }
                }
                Err(e) => return Err(nice(&e)),
            }
        }
    }
    let cport = if port.trim().is_empty() { host.trim() } else { port.trim() };
    let mut body = ContainerCreateBody { image: Some(image.to_string()), ..Default::default() };
    if !cport.is_empty() {
        let key = format!("{cport}/tcp");
        body.exposed_ports = Some(vec![key.clone()]);
        if !host.trim().is_empty() {
            let mut map = HashMap::new();
            map.insert(key, Some(vec![PortBinding { host_ip: None, host_port: Some(host.trim().to_string()) }]));
            body.host_config = Some(HostConfig { port_bindings: Some(map), ..Default::default() });
        }
    }
    d.create_container(Some(CreateContainerOptionsBuilder::new().name(name).build()), body).await.map_err(|e| nice(&e))?;
    if let Err(e) = d.start_container(name, None).await {
        let _ = d.remove_container(name, Some(RemoveContainerOptionsBuilder::new().force(true).build())).await;
        let msg = nice(&e);
        if msg.contains("address already in use") || msg.contains("port is already allocated") {
            return Err(format!("Port {} is already used by another program on this Mac. Choose another port.", host.trim()));
        }
        return Err(msg);
    }
    Ok(())
}

async fn pull(d: &Docker, image: &str, out: &Out) {
    let (from, tag) = split_ref(image);
    let mut opts = CreateImageOptionsBuilder::new().from_image(&from);
    if let Some(t) = &tag {
        opts = opts.tag(t);
    }
    let mut s = d.create_image(Some(opts.build()), None, None);
    let mut order: Vec<String> = Vec::new();
    let mut frac: HashMap<String, f64> = HashMap::new();
    let mut last = Instant::now() - Duration::from_secs(1);
    let send = |order: &Vec<String>, frac: &HashMap<String, f64>, done: bool, err: Option<String>| {
        let layers = order.iter().map(|id| (id.clone(), frac.get(id).copied().unwrap_or(0.0))).collect();
        out.send(Ev::Pull { image: image.to_string(), layers, done, err });
    };
    while let Some(item) = s.next().await {
        let info = match item {
            Ok(i) => i,
            Err(e) => {
                send(&order, &frac, false, Some(nice(&e)));
                return;
            }
        };
        if let Some(e) = info.error_detail.and_then(|e| e.message) {
            send(&order, &frac, false, Some(sentence(&e)));
            return;
        }
        let (Some(id), Some(status)) = (info.id, info.status) else { continue };
        let is_layer = id.len() == 12 && id.chars().all(|c| c.is_ascii_hexdigit());
        if !is_layer {
            continue;
        }
        if !order.contains(&id) {
            order.push(id.clone());
        }
        let pd = info.progress_detail.unwrap_or_default();
        let part = match (pd.current, pd.total) {
            (Some(c), Some(t)) if t > 0 => (c as f64 / t as f64).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let v = match status.as_str() {
            "Downloading" => 0.8 * part,
            "Verifying Checksum" | "Download complete" => 0.8,
            "Extracting" => 0.8 + 0.2 * part,
            "Pull complete" | "Already exists" => 1.0,
            _ => frac.get(&id).copied().unwrap_or(0.0),
        };
        let e = frac.entry(id).or_insert(0.0);
        *e = e.max(v);
        if last.elapsed() > Duration::from_millis(120) {
            last = Instant::now();
            send(&order, &frac, false, None);
        }
    }
    for v in frac.values_mut() {
        *v = 1.0;
    }
    send(&order, &frac, true, None);
    out.refresh();
}

/// Print backend errors when PATINA_DEBUG is set.
fn debug<T>(what: &str, r: &Result<T, DockerError>) {
    if let Err(e) = r {
        if std::env::var_os("PATINA_DEBUG").is_some() {
            eprintln!("[patina] {what}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_build_history() {
        let text = r#"{"cached_steps":12,"completed_at":"2026-11-25T17:40:05.5Z","completed_steps":23,"created_at":"2026-11-25T17:39:01.5Z","name":"shop/api","ref":"patina/patina/ui9vigsfu1fnzimhafuwc7po8","status":"Completed","total_steps":23}
{"created_at":"2026-11-25T17:41:00Z","name":"shop/web","ref":"patina/patina/abc","status":"Running","total_steps":9,"completed_steps":4}
{"created_at":"2026-11-25T17:30:00Z","completed_at":"2026-11-25T17:30:10Z","name":"shop/worker","ref":"patina/patina/def","status":"Error","error":"exit code: 1"}
not json"#;
        let b = parse_builds(text);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].id, "ui9vigsfu1fnzimhafuwc7po8");
        assert_eq!((b[0].st, b[0].steps, b[0].cached), (BuildSt::Done, 23, 12));
        assert!((b[0].secs - 64.0).abs() < 0.01);
        assert_eq!((b[1].st, b[1].secs, b[1].done_steps), (BuildSt::Running, 0.0, 4));
        assert_eq!((b[2].st, b[2].err.as_str()), (BuildSt::Failed, "exit code: 1"));
    }

    #[test]
    fn reads_build_details() {
        let json = r#"{"Name":"shop/api","Context":"code/shop","Dockerfile":"services/api/Dockerfile","VCSRepository":"git@github.com:acme/shop.git","VCSRevision":"53cb3fa2","Platform":["linux/amd64","linux/arm64"],"BuildArgs":[{"Name":"APP_VERSION","Value":"1.2"}],"Status":"completed"}"#;
        let i = parse_build_info(json);
        assert_eq!(i.dockerfile, "services/api/Dockerfile");
        assert_eq!(i.platform, "linux/amd64, linux/arm64");
        assert_eq!(i.args, "APP_VERSION=1.2");
        assert!(i.err.is_empty());
        assert_eq!(parse_build_info(r#"{"Error":{"Message":"exit code: 1"}}"#).err, "exit code: 1");
    }

    #[test]
    fn splits_image_references() {
        assert_eq!(split_ref("nginx"), ("nginx".into(), Some("latest".into())));
        assert_eq!(split_ref("nginx:1.27-alpine"), ("nginx".into(), Some("1.27-alpine".into())));
        assert_eq!(split_ref("localhost:5000/app"), ("localhost:5000/app".into(), Some("latest".into())));
        assert_eq!(split_ref("ghcr.io/o/app:v2"), ("ghcr.io/o/app".into(), Some("v2".into())));
    }

    #[test]
    fn formats_ages() {
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(3600 * 2), "2 hours ago");
        assert_eq!(ago(86_400), "1 day ago");
        assert_eq!(ago(86_400 * 21), "3 weeks ago");
    }

    #[test]
    fn parses_log_lines() {
        let l = log_lines(b"2025-01-01T10:00:00.5Z \x1b[32mserver listening on :80\x1b[0m\n");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].1, "server listening on :80");
        assert_eq!(l[0].2, 1);
    }

    #[test]
    fn writes_messages_as_sentences() {
        assert_eq!(sentence("conflict: unable to remove"), "Conflict: unable to remove.");
    }
}
