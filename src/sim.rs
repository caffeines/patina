//! The Patina engine model.
//!
//! This is a simulated engine, ported from the design prototype: containers
//! start and stop, CPU and memory move, logs stream, images pull layer by
//! layer, and Kubernetes boots its system pods. Nothing here touches a real
//! Docker daemon. Every UI action goes through a method on [`State`], so a
//! real backend can replace this module without changing the views.

use std::collections::{HashMap, HashSet};

use crate::live::{self, Cmd as LCmd, EngineOp};

pub type Job = Box<dyn FnOnce(&mut State)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum St {
    Running,
    Stopped,
    Starting,
    Stopping,
    Paused,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Containers,
    Images,
    Volumes,
    Networks,
    Machines,
    Kubernetes,
    Settings,
}

impl View {
    pub const ALL: [View; 7] =
        [View::Containers, View::Images, View::Volumes, View::Networks, View::Machines, View::Kubernetes, View::Settings];

    pub fn label(self) -> &'static str {
        match self {
            View::Containers => "Containers",
            View::Images => "Images",
            View::Volumes => "Volumes",
            View::Networks => "Networks",
            View::Machines => "Machines",
            View::Kubernetes => "Kubernetes",
            View::Settings => "Settings",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            View::Containers => "containers",
            View::Images => "images",
            View::Volumes => "volumes",
            View::Networks => "networks",
            View::Machines => "machines",
            View::Kubernetes => "kubernetes",
            View::Settings => "settings",
        }
    }

    pub fn parse(s: &str) -> Option<View> {
        View::ALL.into_iter().find(|v| v.slug() == s)
    }

    pub fn index(self) -> usize {
        View::ALL.iter().position(|v| *v == self).unwrap_or(0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    All,
    Running,
    Stopped,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    General,
    Resources,
    Network,
    Engine,
    About,
}

impl Pane {
    pub const ALL: [Pane; 5] = [Pane::General, Pane::Resources, Pane::Network, Pane::Engine, Pane::About];
    pub fn label(self) -> &'static str {
        match self {
            Pane::General => "General",
            Pane::Resources => "Resources",
            Pane::Network => "Network",
            Pane::Engine => "Engine",
            Pane::About => "About",
        }
    }
    pub fn parse(s: &str) -> Option<Pane> {
        Pane::ALL.into_iter().find(|p| p.label().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Logs,
    Term,
    Info,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Engine {
    Running,
    /// Just booted; containers are resuming.
    Ready,
    Booting,
    Off,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kube {
    Off,
    Starting,
    On,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Upd {
    Idle,
    Checking,
    Current,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ov {
    Run,
    Modal,
    Machine,
    Palette,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Banner {
    Update,
    Disk,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Imgs,
    Vols,
    Nets,
    Ms,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Undo {
    Box(String),
    Asset(Kind, String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModalKind {
    Delete,
    DelImg,
    DelVol,
    DelNet,
    DelMachine,
    Blocked,
    VolBlocked,
    NetBlocked,
    Prune,
    Volume,
    Network,
    Reset,
    Pull,
}

#[derive(Clone, Debug)]
pub struct Container {
    pub id: String,
    pub image: String,
    pub port: String,
    pub grp: String,
    pub st: St,
    pub base: f64,
    pub m0: f64,
    pub mem: f64,
    pub cpu: f64,
    pub hex: String,
    pub up: String,
    pub bars: [f32; 12],
    pub tag: String,
    pub why: String,
    pub bad: bool,
    pub loops: u32,
    pub was: bool,
    pub gone: bool,
    pub added: bool,
    /// When the container last turned green (drives the patina pulse).
    pub fresh_at: f64,
    /// When the state last changed (restarts terminal animations).
    pub since: f64,
    /// When it was created in this session (entrance animation).
    pub born: f64,
    pub ports_text: String,
    pub net: String,
    pub restart: String,
    pub hostname: String,
    /// Memory limit in MB (for the detail bar).
    pub mem_limit: f64,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub t: String,
    pub m: String,
    /// 0 plain, 1 good, 2 warning, 3 error
    pub k: u8,
}

#[derive(Clone, Debug)]
pub struct Asset {
    pub n: String,
    pub mb: f64,
    pub used_by: String,
    pub age: String,
    pub fresh: bool,
    pub gone: bool,
    pub born: f64,
}

#[derive(Clone, Debug)]
pub struct Net {
    pub n: String,
    pub driver: String,
    pub subnet: String,
    pub sys: bool,
    pub grp: String,
    pub fresh: bool,
    pub gone: bool,
    pub born: f64,
    /// Containers on this network (live engine); `None` means "work it out from the group".
    pub attached: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Machine {
    pub id: String,
    pub distro: String,
    pub st: St,
    pub up: String,
    pub was: bool,
    pub gone: bool,
    pub added: bool,
    pub fresh_at: f64,
    pub since: f64,
    pub born: f64,
    pub arch: String,
}

#[derive(Clone, Debug)]
pub struct Pod {
    pub id: String,
    pub st: St,
    pub fresh_at: f64,
    pub shown: bool,
    pub shown_at: f64,
    /// Live readiness ("1/1") and phase; empty in the simulation.
    pub ready: String,
    pub phase: String,
}

#[derive(Clone, Debug)]
pub struct Pull {
    pub image: String,
    /// (layer id, progress 0..1, size label)
    pub layers: Vec<(String, f64, String)>,
    pub done: bool,
    pub err: bool,
    pub msg: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pending {
    St(St),
    Gone,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub login: bool,
    pub menubar: bool,
    pub autostart: bool,
    pub autoupdate: bool,
    pub ports: bool,
    pub dns: bool,
    pub rosetta: bool,
    pub cpus: u32,
    pub mem: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Modal {
    pub kind: Option<ModalKind>,
    pub target: String,
    pub value: String,
    pub err: String,
}

#[derive(Clone, Debug, Default)]
pub struct RunForm {
    pub image: String,
    pub name: String,
    pub host: String,
    pub port: String,
    pub err: String,
    pub perr: String,
    pub busy: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MkForm {
    pub pick: usize,
    pub name: String,
    pub busy: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Pal {
    pub q: String,
    pub i: usize,
}

pub struct Toast {
    pub msg: String,
    pub act: String,
    pub undo: Option<Undo>,
    pub at: f64,
}

pub struct Cmd {
    pub label: String,
    pub hint: &'static str,
    pub run: Job,
}

pub struct ModalSpec {
    pub title: String,
    pub body: String,
    pub field: bool,
    pub label: &'static str,
    pub ph: &'static str,
    pub yes: String,
    pub no: &'static str,
    pub danger: bool,
    pub go: Job,
}

/// Which design board to start on. Mirrors the prototype's props.
#[derive(Clone, Debug, Default)]
pub struct Preset {
    pub view: String,
    pub pane: String,
    pub scenario: String,
    pub show: String,
    pub phase: String,
}

pub const DISTROS: [(&str, &str, &str); 5] = [
    ("ubuntu", "Ubuntu 24.04 LTS", "U"),
    ("debian", "Debian 13", "D"),
    ("fedora", "Fedora", "F"),
    ("arch", "Arch Linux", "A"),
    ("alpine", "Alpine", "Al"),
];

pub const RUN_PICKS: [&str; 3] = ["nginx:1.27-alpine", "postgres:17", "redis:7.4-alpine"];
pub const PULL_IMAGE: &str = "nginx:1.27-alpine";
pub const LAYERS: [(&str, &str); 4] = [("f182321", "3.6 MB"), ("61ca4f7", "1.8 MB"), ("b464cfd", "12.4 MB"), ("d7e5070", "0.6 MB")];
pub const PODS: [&str; 5] = ["coredns", "local-path-provisioner", "metrics-server", "traefik", "svclb-traefik"];
const IDLE_BAR: f32 = 0.08;

pub fn lv(cpu: f64) -> f32 {
    (0.08 + cpu / 9.0).min(1.0) as f32
}

pub fn fmt_clock(t: u64) -> String {
    format!("{:02}:{:02}:{:02}", (t / 3600) % 24, (t / 60) % 60, t % 60)
}

pub fn fmt_size(mb: f64) -> String {
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{} MB", mb.round() as i64)
    }
}

pub fn fmt_count(v: usize, word: &str) -> String {
    format!("{} {}{}", v, word, if v == 1 { "" } else { "s" })
}

fn log_lines(id: &str) -> &'static [(&'static str, u8)] {
    match id {
        "web" => &[
            ("GET /products 200 18ms", 0),
            ("GET /cart 200 11ms", 0),
            ("compiled /checkout 240ms", 1),
            ("GET /api/cart 200 6ms", 0),
            ("hot reload cart/page.tsx", 0),
            ("GET /products/82 200 14ms", 0),
        ],
        "api" => &[
            ("POST /v1/orders 201 12ms", 1),
            ("GET /v1/products 200 7ms", 0),
            ("pool 4 idle, 1 active", 0),
            ("cache hit products:featured", 0),
            ("GET /v1/cart/19 200 5ms", 0),
            ("slow query 312ms orders_by_user", 2),
        ],
        "db" => &[
            ("checkpoint complete, 42 buffers", 0),
            ("connection authorized user=app", 0),
            ("autovacuum table \"orders\"", 0),
            ("checkpoint starting: time", 0),
        ],
        "cache" => &[
            ("10 changes in 300 s, saving", 0),
            ("background saving started", 0),
            ("DB saved on disk", 1),
            ("background saving finished", 0),
        ],
        "worker" => &[
            ("job email.receipt done 84ms", 1),
            ("job stock.sync done 1.2s", 1),
            ("polling queue default", 0),
            ("job image.resize done 310ms", 1),
        ],
        "mailpit" => &[("smtp received 1.2 kB", 0), ("GET /api/v1/messages 200", 0), ("smtp connection 172.18.0.4", 0)],
        "minio" => &[("API http://172.18.0.7:9000", 0), ("scan uploads: 3 objects", 0), ("heal check: nothing to heal", 1)],
        _ => &[("listening on 0.0.0.0", 1), ("GET /health 200 2ms", 0), ("request served in 9ms", 0)],
    }
}

pub struct State {
    pub time: f64,
    timers: Vec<(f64, u64, Job)>,
    seq: u64,
    rng: u64,
    next_pulse: f64,
    pub preset: Preset,

    pub view: View,
    pub view_at: f64,
    pub filter: Filter,
    pub q: String,
    pub sel: String,
    pub panel_at: f64,
    pub tab: Tab,
    pub tab_at: f64,
    pub log_at: f64,

    pub engine: Engine,
    pub boot: usize,
    pub cold: bool,
    pub show_log: bool,
    pub clock: u64,

    pub cs: Vec<Container>,
    pub logs: HashMap<String, Vec<LogLine>>,
    pub imgs: Vec<Asset>,
    pub vols: Vec<Asset>,
    pub nets: Vec<Net>,
    pub ms: Vec<Machine>,
    pub pods: Vec<Pod>,
    pub kube: Kube,
    pub kstep: usize,

    pub pull: Option<Pull>,
    pub pulled: bool,
    pull_gen: u32,
    pull_turn: u32,

    pub set: Settings,
    pub saved: (u32, u32),
    pub pane: Pane,
    pub upd: Upd,
    pub upd_at: f64,

    pub ov: Option<Ov>,
    pub ov_out: bool,
    pub ov_at: f64,
    ov_gen: u32,
    pub modal: Modal,
    pub run: RunForm,
    pub mk: MkForm,
    pub pal: Pal,
    pub menu: Option<String>,
    pub menu_at: f64,
    pub toast: Option<Toast>,
    pub tk: u32,
    pub banner: Option<Banner>,
    /// A text field to focus on the next frame.
    pub focus: Option<&'static str>,

    // ---- live engine
    pub live: Option<live::Handle>,
    pub pending: HashMap<String, Pending>,
    pub pending_m: HashMap<String, St>,
    pub pending_a: HashSet<String>,
    pub pending_new: HashSet<String>,
    pub engine_op: Option<EngineOp>,
    op_saw_down: bool,
    pub boot_at: f64,
    pub ready_secs: f64,
    pub info: live::Info,
    /// The engine has a disk image (setup has run).
    pub installed: bool,
    /// The engine's own state while Docker isn't reachable yet.
    pub engine_state: String,
    pub engine_msg: String,
    /// The app should show setup (onboarding).
    pub want_setup: bool,
    pub endpoint: String,
    pub machines_ok: bool,
    pub cfg: HashMap<String, String>,
    pub engine_log: String,
    pub urls: Vec<String>,
    sent_sel: String,
    seen_boxes: bool,
    seen_assets: bool,
    kube_busy: bool,
}

impl State {
    pub fn new(p: Preset, now: f64) -> State {
        let sc = p.scenario.as_str();
        let show = p.show.as_str();
        let phase = p.phase.as_str();
        let view = View::parse(&p.view).unwrap_or(View::Containers);
        let pane = Pane::parse(&p.pane).unwrap_or(Pane::General);
        let bare = sc == "empty";
        let seed = (now.to_bits() ^ 0x9E37_79B9_7F4A_7C15) | 1;

        let mut s = State {
            time: now,
            timers: Vec::new(),
            seq: 0,
            rng: seed,
            next_pulse: now + 1.1,
            preset: p.clone(),
            view,
            view_at: now,
            filter: Filter::All,
            q: if phase == "nomatch" { "kafka".into() } else { String::new() },
            sel: if sc == "trouble" { "api".into() } else { "web".into() },
            panel_at: now,
            tab: Tab::Logs,
            tab_at: now,
            log_at: -10.0,
            engine: match sc {
                "stopped" => Engine::Off,
                "failed" => Engine::Failed,
                _ => Engine::Running,
            },
            boot: 0,
            cold: false,
            show_log: false,
            clock: 50531,
            cs: Vec::new(),
            logs: HashMap::new(),
            imgs: Vec::new(),
            vols: Vec::new(),
            nets: Vec::new(),
            ms: Vec::new(),
            pods: Vec::new(),
            kube: Kube::Off,
            kstep: 0,
            pull: None,
            pulled: false,
            pull_gen: 0,
            pull_turn: 0,
            set: Settings {
                login: true,
                menubar: true,
                autostart: true,
                autoupdate: false,
                ports: true,
                dns: true,
                rosetta: true,
                cpus: 4,
                mem: 8,
            },
            saved: (4, 8),
            pane,
            upd: Upd::Idle,
            upd_at: -10.0,
            ov: None,
            ov_out: false,
            ov_at: now,
            ov_gen: 0,
            modal: Modal::default(),
            run: RunForm::default(),
            mk: MkForm::default(),
            pal: Pal::default(),
            menu: None,
            menu_at: now,
            toast: None,
            tk: 0,
            banner: match sc {
                "update" => Some(Banner::Update),
                "diskfull" => Some(Banner::Disk),
                _ => None,
            },
            focus: None,
            live: None,
            pending: HashMap::new(),
            pending_m: HashMap::new(),
            pending_a: HashSet::new(),
            pending_new: HashSet::new(),
            engine_op: None,
            op_saw_down: false,
            boot_at: now,
            ready_secs: 1.7,
            info: live::Info::default(),
            installed: true,
            engine_state: String::new(),
            engine_msg: String::new(),
            want_setup: false,
            endpoint: String::new(),
            machines_ok: true,
            cfg: HashMap::new(),
            engine_log: String::new(),
            urls: Vec::new(),
            sent_sel: String::new(),
            seen_boxes: false,
            seen_assets: false,
            kube_busy: false,
        };

        let mut cs = vec![
            s.mk_box("web", "node:22-alpine", "3000", "storefront", St::Running, 3.4, 184.0, "a41f9c2e07b3", "2h 14m"),
            s.mk_box("api", "storefront/api:dev", "8080", "storefront", St::Running, 5.8, 212.0, "5d0e7b1c94aa", "2h 14m"),
            s.mk_box("db", "postgres:17", "5432", "storefront", St::Running, 1.2, 96.0, "c7729e04d1f6", "3d 4h"),
            s.mk_box("cache", "redis:7.4-alpine", "6379", "storefront", St::Running, 0.4, 12.0, "0b3a6f58e2c1", "3d 4h"),
            s.mk_box("worker", "storefront/worker:dev", "", "storefront", St::Stopped, 2.1, 148.0, "e19d4a7730b5", ""),
            s.mk_box("mailpit", "axllent/mailpit", "8025", "standalone", St::Running, 0.2, 24.0, "8f62c0d91e47", "6d 1h"),
            s.mk_box("minio", "minio/minio", "9000", "standalone", St::Stopped, 0.9, 132.0, "37b8e5a2c60d", ""),
        ];
        if sc == "trouble" {
            for c in cs.iter_mut() {
                match c.id.as_str() {
                    "api" => {
                        c.st = St::Error;
                        c.cpu = 0.0;
                        c.bars = [IDLE_BAR; 12];
                        c.tag = "exited 137".into();
                        c.why = "Killed by the kernel: out of memory. The limit is 256 MB.".into();
                    }
                    "db" => {
                        c.tag = "unhealthy".into();
                        c.bad = true;
                        c.why = "The health check failed 3 times: pg_isready timed out.".into();
                    }
                    "cache" => {
                        c.st = St::Paused;
                        c.cpu = 0.0;
                    }
                    "worker" => {
                        c.st = St::Starting;
                        c.loops = 3;
                        c.why = "Exited with code 1 and is being restarted. Attempt 3.".into();
                    }
                    "minio" => {
                        c.st = St::Error;
                        c.tag = "port 9000 in use".into();
                        c.why = "Port 9000 is already used by another program on this Mac.".into();
                    }
                    _ => {}
                }
            }
        }
        if bare {
            cs.clear();
        }
        if matches!(sc, "stopped" | "failed" | "loading") {
            for c in cs.iter_mut().filter(|c| c.st == St::Running) {
                c.st = St::Stopped;
                c.cpu = 0.0;
                c.was = true;
                c.bars = [IDLE_BAR; 12];
            }
        }
        if show == "toast" {
            if let Some(c) = cs.iter_mut().find(|c| c.id == "worker") {
                c.gone = true;
            }
        }
        for c in &cs {
            let log = s.seed_log(c);
            s.logs.insert(c.id.clone(), log);
        }
        s.cs = cs;

        let a = |n: &str, mb: f64, used_by: &str, age: &str| Asset {
            n: n.into(),
            mb,
            used_by: used_by.into(),
            age: age.into(),
            fresh: false,
            gone: false,
            born: -10.0,
        };
        if !bare {
            s.imgs = vec![
                a("storefront/api:dev", 412.0, "api", "2 hours ago"),
                a("storefront/worker:dev", 398.0, "worker", "2 hours ago"),
                a("node:22-alpine", 158.0, "web", "5 days ago"),
                a("postgres:17", 438.0, "db", "2 weeks ago"),
                a("redis:7.4-alpine", 41.0, "cache", "2 weeks ago"),
                a("axllent/mailpit", 29.0, "mailpit", "3 weeks ago"),
                a("minio/minio", 176.0, "minio", "1 month ago"),
                a("adminer:4", 118.0, "adminer", "1 month ago"),
                a("ubuntu:24.04", 78.0, "", "1 month ago"),
                a("alpine:3.20", 8.0, "", "2 months ago"),
            ];
            s.vols = vec![
                a("storefront_pgdata", 1400.0, "db", "3 weeks ago"),
                a("storefront_uploads", 640.0, "api", "3 weeks ago"),
                a("minio_data", 212.0, "minio", "1 month ago"),
                a("mailpit_data", 38.0, "mailpit", "3 weeks ago"),
                a("storefront_redis", 12.0, "cache", "3 weeks ago"),
            ];
        }
        let net = |n: &str, driver: &str, subnet: &str, sys: bool, grp: &str| Net {
            n: n.into(),
            driver: driver.into(),
            subnet: subnet.into(),
            sys,
            grp: grp.into(),
            fresh: false,
            gone: false,
            born: -10.0,
            attached: None,
        };
        s.nets = vec![
            net("bridge", "bridge", "172.17.0.0/16", true, "standalone"),
            net("storefront_default", "bridge", "172.18.0.0/16", false, "storefront"),
            net("host", "host", "", true, ""),
            net("none", "null", "", true, ""),
        ];
        if !bare {
            s.ms = vec![
                Machine {
                    id: "ubuntu".into(),
                    distro: "Ubuntu 24.04 LTS".into(),
                    st: St::Running,
                    up: "up 3d 4h".into(),
                    was: false,
                    gone: false,
                    added: false,
                    fresh_at: -10.0,
                    since: -10.0,
                    born: -10.0,
                    arch: "arm64".into(),
                },
                Machine {
                    id: "debian".into(),
                    distro: "Debian 13".into(),
                    st: St::Stopped,
                    up: String::new(),
                    was: false,
                    gone: false,
                    added: false,
                    fresh_at: -10.0,
                    since: -10.0,
                    born: -10.0,
                    arch: "arm64".into(),
                },
            ];
        }
        let k_on = view == View::Kubernetes && phase == "on";
        let k_busy = view == View::Kubernetes && phase == "busy";
        s.pods = PODS
            .iter()
            .map(|id| Pod {
                id: (*id).into(),
                st: if k_on { St::Running } else { St::Starting },
                fresh_at: -10.0,
                shown: k_on,
                shown_at: -10.0,
                ready: String::new(),
                phase: String::new(),
            })
            .collect();
        if k_on {
            s.kube = Kube::On;
        }

        if view == View::Settings && phase == "dirty" {
            s.set.cpus = 6;
            s.set.mem = 12;
        }

        // Overlays the prototype could start on.
        match show {
            "run" => {
                s.open_run("");
                match phase {
                    "invalid" => s.run.err = "Enter an image, for example nginx:1.27-alpine.".into(),
                    "conflict" => {
                        s.run.image = PULL_IMAGE.into();
                        s.run.port = "80".into();
                        s.run.host = "3000".into();
                        s.run.perr = "Port 3000 is already used by web. Choose another port.".into();
                    }
                    "busy" => {
                        s.run.image = PULL_IMAGE.into();
                        s.run.port = "80".into();
                        s.run.host = "8088".into();
                        s.run_go();
                    }
                    _ => {}
                }
            }
            "delete" => s.ask_modal(ModalKind::Delete, "worker"),
            "blocked" => {
                s.view = View::Images;
                s.ask_modal(ModalKind::Blocked, "node:22-alpine");
            }
            "prune" => {
                s.view = View::Images;
                s.ask_modal(ModalKind::Prune, "");
            }
            "volume" => {
                s.view = View::Volumes;
                s.ask_modal(ModalKind::Volume, "");
                if phase == "invalid" {
                    s.modal.value = "storefront_pgdata".into();
                    s.modal.err = "A volume named storefront_pgdata already exists.".into();
                }
            }
            "machine" => {
                s.view = View::Machines;
                s.open_mk();
            }
            "palette" => {
                s.open_pal();
                if phase == "invalid" {
                    s.pal.q = "backup".into();
                }
            }
            "menu" => s.toggle_menu("api"),
            "toast" => s.toast_say("Deleted worker", "Undo", Some(Undo::Box("worker".into()))),
            "pull" => {
                s.view = View::Images;
                s.pull_image();
            }
            _ => {}
        }
        if sc == "loading" {
            s.engine = Engine::Off;
            s.cold_boot();
        }
        if k_busy {
            s.flip_kube();
        }
        s
    }

    // ---------------------------------------------------------------- utils

    pub fn rnd(&mut self) -> f64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn seed(&mut self, base: f64) -> [f32; 12] {
        let mut out = [IDLE_BAR; 12];
        if base > 0.0 {
            for b in out.iter_mut() {
                let r = self.rnd();
                *b = lv(base * (0.5 + r));
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn mk_box(&mut self, id: &str, image: &str, port: &str, grp: &str, st: St, base: f64, m0: f64, hex: &str, up: &str) -> Container {
        let running = st == St::Running;
        Container {
            id: id.into(),
            image: image.into(),
            port: port.into(),
            grp: grp.into(),
            st,
            base,
            m0,
            mem: m0,
            cpu: if running { base } else { 0.0 },
            hex: hex.into(),
            up: up.into(),
            bars: self.seed(if running { base } else { 0.0 }),
            tag: String::new(),
            why: String::new(),
            bad: false,
            loops: 0,
            was: false,
            gone: false,
            added: false,
            fresh_at: -10.0,
            since: -10.0,
            born: -10.0,
            ports_text: String::new(),
            net: String::new(),
            restart: "unless-stopped".into(),
            hostname: String::new(),
            mem_limit: 256.0,
        }
    }

    fn seed_log(&self, c: &Container) -> Vec<LogLine> {
        let lines = log_lines(&c.id);
        let mut t: u64 = 50490;
        let mut out: Vec<LogLine> = (0..7)
            .map(|i| {
                t += 3 + (i as u64 * 7) % 5;
                let (m, k) = lines[i % lines.len()];
                LogLine { t: fmt_clock(t), m: m.into(), k }
            })
            .collect();
        let tail: Option<&[(&str, u8)]> = match c.id.as_str() {
            "api" => Some(&[("FATAL heap limit reached", 3), ("exited with code 137", 3)]),
            "minio" => Some(&[("bind 0.0.0.0:9000: address in use", 3)]),
            "worker" => Some(&[("Error: REDIS_URL is not set", 3), ("exited with code 1", 3)]),
            "db" => Some(&[("health check: pg_isready timed out", 2)]),
            _ => None,
        };
        match tail {
            Some(tail) if !c.why.is_empty() => {
                for (i, (m, k)) in tail.iter().enumerate() {
                    out.push(LogLine { t: fmt_clock(t + 2 + i as u64), m: (*m).into(), k: *k });
                }
            }
            _ => {
                if c.st == St::Stopped && !c.was {
                    out.push(LogLine { t: fmt_clock(t + 2), m: "exited with code 0".into(), k: 0 });
                }
            }
        }
        let n = out.len();
        out.split_off(n.saturating_sub(10))
    }

    /// Run `f` after `ms` milliseconds (the prototype's `setTimeout`).
    pub fn after(&mut self, ms: u64, f: impl FnOnce(&mut State) + 'static) {
        self.seq += 1;
        self.timers.push((self.time + ms as f64 / 1000.0, self.seq, Box::new(f)));
    }

    pub fn next_due(&self) -> f64 {
        self.timers.iter().map(|t| t.0).fold(self.next_pulse, f64::min)
    }

    /// Advance the simulation to `now`: fire due timers and the 1.1 s pulse.
    pub fn tick(&mut self, now: f64) {
        self.time = now;
        if let Some(live) = &self.live {
            for ev in live.drain() {
                self.apply(ev);
            }
            if self.sel != self.sent_sel {
                self.sent_sel = self.sel.clone();
                self.send(LCmd::Select(self.sel.clone()));
            }
            self.next_pulse = now + 1.1;
        } else if now >= self.next_pulse {
            self.pulse();
            self.next_pulse = now + 1.1;
        }
        for _ in 0..16 {
            let mut due = Vec::new();
            let mut i = 0;
            while i < self.timers.len() {
                if self.timers[i].0 <= now {
                    due.push(self.timers.swap_remove(i));
                } else {
                    i += 1;
                }
            }
            if due.is_empty() {
                break;
            }
            due.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for (_, _, job) in due {
                job(self);
            }
        }
    }

    fn send(&self, cmd: LCmd) {
        if let Some(l) = &self.live {
            l.send(cmd);
        }
    }

    /// Show a container in a transitional state until the engine confirms.
    fn pend(&mut self, id: &str, st: St) {
        self.pending.insert(id.to_string(), Pending::St(st));
        self.patch(id, |c| c.st = st);
    }

    fn patch(&mut self, id: &str, f: impl FnOnce(&mut Container)) {
        let now = self.time;
        if let Some(c) = self.cs.iter_mut().find(|c| c.id == id) {
            let before = c.st;
            f(c);
            if c.st != before {
                c.since = now;
            }
        }
    }

    fn patch_machine(&mut self, id: &str, f: impl FnOnce(&mut Machine)) {
        let now = self.time;
        if let Some(m) = self.ms.iter_mut().find(|m| m.id == id) {
            let before = m.st;
            f(m);
            if m.st != before {
                m.since = now;
            }
        }
    }

    fn patch_pod(&mut self, id: &str, f: impl FnOnce(&mut Pod)) {
        if let Some(p) = self.pods.iter_mut().find(|p| p.id == id) {
            f(p);
        }
    }

    pub fn alive(&self) -> impl Iterator<Item = &Container> {
        self.cs.iter().filter(|c| !c.gone)
    }

    pub fn live(&self) -> bool {
        matches!(self.engine, Engine::Running | Engine::Ready)
    }

    fn add_log(&mut self, id: &str, t: u64, line: Option<(String, u8)>) {
        let (m, k) = match line {
            Some(x) => x,
            None => {
                let lines = log_lines(id);
                let i = (self.rnd() * lines.len() as f64) as usize % lines.len();
                (lines[i].0.to_string(), lines[i].1)
            }
        };
        let list = self.logs.entry(id.to_string()).or_default();
        list.push(LogLine { t: fmt_clock(t), m, k });
        let n = list.len();
        if n > 10 {
            list.drain(0..n - 10);
        }
    }

    fn log_note(&mut self, id: &str, m: impl Into<String>, k: u8) {
        let clock = self.clock;
        self.add_log(id, clock, Some((m.into(), k)));
        if id == self.sel {
            self.log_at = self.time;
        }
    }

    // ---------------------------------------------------------------- pulse

    fn pulse(&mut self) {
        if !self.live() {
            return;
        }
        for i in 0..self.cs.len() {
            let st = self.cs[i].st;
            if st == St::Paused {
                continue;
            }
            if st != St::Running {
                let c = &mut self.cs[i];
                if !c.bars.iter().all(|&b| b == IDLE_BAR) {
                    c.cpu = 0.0;
                    c.bars.rotate_left(1);
                    c.bars[11] = IDLE_BAR;
                }
                continue;
            }
            let r1 = self.rnd();
            let r2 = self.rnd();
            let c = &mut self.cs[i];
            let cpu = (c.base * (0.5 + r1)).max(0.1);
            c.cpu = cpu;
            c.mem = (c.m0 * (0.97 + r2 * 0.06)).round();
            c.bars.rotate_left(1);
            c.bars[11] = lv(cpu);
        }
        self.clock += 1;
        let cur = self.cs.iter().find(|c| c.id == self.sel).map(|c| (c.id.clone(), c.st == St::Running && !c.gone));
        if let Some((id, true)) = cur {
            if self.rnd() < 0.8 {
                let clock = self.clock;
                self.add_log(&id, clock, None);
                self.log_at = self.time;
            }
        }
    }

    // ------------------------------------------------------------- navigate

    pub fn go_view(&mut self, view: View) {
        if view != self.view {
            self.view = view;
            self.view_at = self.time;
            self.q.clear();
            self.menu = None;
        }
    }

    pub fn pick(&mut self, id: &str) {
        if id != self.sel {
            self.sel = id.to_string();
            self.panel_at = self.time;
            self.menu = None;
        }
    }

    pub fn toggle_menu(&mut self, id: &str) {
        if self.menu.as_deref() == Some(id) {
            self.menu = None;
        } else {
            self.menu = Some(id.to_string());
            self.menu_at = self.time;
        }
    }

    pub fn set_tab(&mut self, tab: Tab) {
        if tab != self.tab {
            self.tab = tab;
            self.tab_at = self.time;
        }
    }

    pub fn set_pane(&mut self, pane: Pane) {
        if pane != self.pane {
            self.pane = pane;
            self.tab_at = self.time;
        }
    }

    pub fn toast_say(&mut self, msg: impl Into<String>, act: &str, undo: Option<Undo>) {
        self.tk += 1;
        let tk = self.tk;
        self.toast = Some(Toast { msg: msg.into(), act: act.into(), undo, at: self.time });
        self.after(4200, move |s| {
            if s.tk == tk {
                s.toast = None;
            }
        });
    }

    pub fn undo_last(&mut self) {
        let Some(t) = self.toast.take() else { return };
        match t.undo {
            Some(Undo::Box(id)) => self.patch(&id, |c| c.gone = false),
            Some(Undo::Asset(kind, id)) => self.set_gone(kind, &id, false),
            None => self.toast = Some(t),
        }
    }

    fn set_gone(&mut self, kind: Kind, id: &str, gone: bool) {
        match kind {
            Kind::Imgs => self.imgs.iter_mut().filter(|a| a.n == id).for_each(|a| a.gone = gone),
            Kind::Vols => self.vols.iter_mut().filter(|a| a.n == id).for_each(|a| a.gone = gone),
            Kind::Nets => self.nets.iter_mut().filter(|a| a.n == id).for_each(|a| a.gone = gone),
            Kind::Ms => self.ms.iter_mut().filter(|a| a.id == id).for_each(|a| a.gone = gone),
        }
    }

    // ------------------------------------------------------------- overlays

    fn open_ov(&mut self, ov: Ov) {
        self.ov = Some(ov);
        self.ov_out = false;
        self.ov_at = self.time;
        self.ov_gen += 1;
        self.menu = None;
    }

    pub fn close_ov(&mut self) {
        if self.ov.is_none() || self.ov_out {
            return;
        }
        self.ov_out = true;
        self.ov_at = self.time;
        let gen = self.ov_gen;
        self.after(170, move |s| {
            if s.ov_gen == gen {
                s.ov = None;
                s.ov_out = false;
                s.modal = Modal::default();
            }
        });
    }

    pub fn open_run(&mut self, image: &str) {
        self.open_ov(Ov::Run);
        self.run = RunForm { image: image.into(), ..Default::default() };
        self.focus = Some("run-image");
    }

    pub fn ask_modal(&mut self, kind: ModalKind, target: &str) {
        self.open_ov(Ov::Modal);
        self.modal = Modal { kind: Some(kind), target: target.into(), value: String::new(), err: String::new() };
        if matches!(kind, ModalKind::Volume | ModalKind::Network | ModalKind::Pull) {
            self.focus = Some("modal-field");
        }
    }

    pub fn open_pal(&mut self) {
        self.open_ov(Ov::Palette);
        self.pal = Pal::default();
        self.focus = Some("palette");
    }

    pub fn open_mk(&mut self) {
        self.open_ov(Ov::Machine);
        self.mk = MkForm::default();
        self.focus = Some("mk-name");
    }

    // ----------------------------------------------------------- containers

    fn start_box(&mut self, id: &str, ms: u64) {
        self.patch(id, |c| {
            c.st = St::Starting;
            c.tag.clear();
            c.why.clear();
            c.loops = 0;
            c.bad = false;
        });
        let id = id.to_string();
        self.after(ms, move |s| {
            let now = s.time;
            s.patch(&id, |c| {
                c.st = St::Running;
                c.fresh_at = now;
                c.up = "just now".into();
            });
            s.log_note(&id, format!("started in {:.1} s", ms as f64 / 1000.0), 1);
        });
    }

    pub fn toggle(&mut self, id: &str) {
        let Some(c) = self.cs.iter().find(|x| x.id == id) else { return };
        if self.engine != Engine::Running {
            return;
        }
        let st = c.st;
        let looping = c.loops > 0;
        if self.live.is_some() {
            if st == St::Running || st == St::Paused || looping {
                self.pend(id, St::Stopping);
                self.send(LCmd::Stop(id.into()));
            } else if st == St::Stopped || st == St::Error {
                self.pend(id, St::Starting);
                self.send(LCmd::Start(id.into()));
            }
            return;
        }
        if st == St::Running || st == St::Paused || looping {
            self.patch(id, |c| {
                c.st = St::Stopping;
                c.loops = 0;
            });
            self.log_note(id, "SIGTERM received, shutting down", 2);
            let id = id.to_string();
            self.after(800, move |s| {
                s.patch(&id, |c| {
                    c.st = St::Stopped;
                    c.cpu = 0.0;
                    c.tag.clear();
                    c.why.clear();
                    c.bad = false;
                })
            });
        } else if st == St::Stopped || st == St::Error {
            self.start_box(id, 1300);
        }
    }

    pub fn restart(&mut self, id: &str) {
        let Some(c) = self.cs.iter().find(|x| x.id == id) else { return };
        if c.st != St::Running || self.engine != Engine::Running {
            return;
        }
        if self.live.is_some() {
            self.pend(id, St::Starting);
            self.send(LCmd::Restart(id.into()));
            return;
        }
        self.patch(id, |c| c.st = St::Stopping);
        let id = id.to_string();
        self.after(600, move |s| s.start_box(&id, 1100));
    }

    pub fn pause_box(&mut self, id: &str) {
        let Some(c) = self.cs.iter().find(|x| x.id == id) else { return };
        let now = self.time;
        if self.live.is_some() {
            match c.st {
                St::Running => {
                    self.pend(id, St::Paused);
                    self.send(LCmd::Pause(id.into()));
                }
                St::Paused => {
                    self.pend(id, St::Running);
                    self.send(LCmd::Unpause(id.into()));
                }
                _ => {}
            }
            return;
        }
        match c.st {
            St::Running => self.patch(id, |c| {
                c.st = St::Paused;
                c.cpu = 0.0;
            }),
            St::Paused => self.patch(id, |c| {
                c.st = St::Running;
                c.fresh_at = now;
            }),
            _ => {}
        }
    }

    pub fn drop_box(&mut self, id: &str) {
        let rest: Vec<String> = self.alive().filter(|c| c.id != id).map(|c| c.id.clone()).collect();
        self.patch(id, |c| {
            c.gone = true;
            c.st = St::Stopped;
            c.cpu = 0.0;
            c.loops = 0;
        });
        if self.sel == id {
            if let Some(first) = rest.first() {
                self.sel = first.clone();
                self.panel_at = self.time;
            }
        }
        if self.live.is_some() {
            self.pending.insert(id.to_string(), Pending::Gone);
            self.send(LCmd::Remove(id.into()));
            return;
        }
        self.toast_say(format!("Deleted {id}"), "Undo", Some(Undo::Box(id.to_string())));
    }

    pub fn group_all(&mut self, grp: &str, stop: bool) {
        if self.engine != Engine::Running {
            return;
        }
        let ids: Vec<String> = self
            .alive()
            .filter(|c| c.grp == grp && if stop { c.st == St::Running } else { matches!(c.st, St::Stopped | St::Error) })
            .map(|c| c.id.clone())
            .collect();
        for (i, id) in ids.into_iter().enumerate() {
            self.after(110 * i as u64, move |s| s.toggle(&id));
        }
    }

    /// Stop every running container (the popover's Stop all).
    pub fn stop_all(&mut self) {
        if self.engine != Engine::Running {
            return;
        }
        let ids: Vec<String> =
            self.alive().filter(|c| matches!(c.st, St::Running | St::Paused) || c.loops > 0).map(|c| c.id.clone()).collect();
        for (i, id) in ids.into_iter().enumerate() {
            self.after(90 * i as u64, move |s| s.toggle(&id));
        }
    }

    // --------------------------------------------------------------- engine

    pub fn reboot(&mut self) {
        if self.engine != Engine::Running {
            return;
        }
        let was: Vec<String> = self.cs.iter().filter(|c| c.st == St::Running).map(|c| c.id.clone()).collect();
        self.engine = Engine::Booting;
        self.boot = 0;
        self.cold = false;
        self.menu = None;
        let now = self.time;
        for c in self.cs.iter_mut().filter(|c| was.contains(&c.id)) {
            c.st = St::Starting;
            c.cpu = 0.0;
            c.bars = [IDLE_BAR; 12];
            c.since = now;
        }
        self.after(560, |s| s.boot = 1);
        self.after(1120, |s| s.boot = 2);
        if self.live.is_some() {
            self.engine_op = Some(EngineOp::Restart);
            self.op_saw_down = false;
            self.boot_at = self.time;
            self.send(LCmd::Engine(EngineOp::Restart));
            return;
        }
        self.after(1700, move |s| {
            s.engine = Engine::Ready;
            for (i, id) in was.into_iter().enumerate() {
                s.after(120 + 160 * i as u64, move |s| {
                    let now = s.time;
                    s.patch(&id, |c| {
                        c.st = St::Running;
                        c.fresh_at = now;
                    });
                });
            }
        });
        self.after(3400, |s| {
            if s.engine == Engine::Ready {
                s.engine = Engine::Running
            }
        });
    }

    pub fn power_flip(&mut self) {
        match self.engine {
            Engine::Running => {
                self.engine = Engine::Off;
                self.menu = None;
                let now = self.time;
                for c in self.cs.iter_mut().filter(|c| matches!(c.st, St::Running | St::Paused)) {
                    c.st = St::Stopped;
                    c.cpu = 0.0;
                    c.was = true;
                    c.bars = [IDLE_BAR; 12];
                    c.since = now;
                }
                for m in self.ms.iter_mut().filter(|m| m.st == St::Running) {
                    m.st = St::Stopped;
                    m.was = true;
                    m.since = now;
                }
                if self.live.is_some() {
                    self.engine_op = Some(EngineOp::Stop);
                    self.send(LCmd::Engine(EngineOp::Stop));
                }
            }
            Engine::Off | Engine::Failed => self.cold_boot(),
            _ => {}
        }
    }

    pub fn cold_boot(&mut self) {
        if self.live.is_some() && !self.installed {
            self.want_setup = true;
            return;
        }
        self.engine = Engine::Booting;
        self.boot = 0;
        self.cold = true;
        self.show_log = false;
        self.after(560, |s| s.boot = 1);
        self.after(1120, |s| s.boot = 2);
        if self.live.is_some() {
            self.engine_op = Some(EngineOp::Start);
            self.boot_at = self.time;
            self.send(LCmd::Engine(EngineOp::Start));
            return;
        }
        self.after(1700, |s| {
            let back: Vec<String> = s.cs.iter().filter(|c| c.was && !c.gone).map(|c| c.id.clone()).collect();
            s.engine = Engine::Ready;
            s.cold = false;
            let now = s.time;
            for c in s.cs.iter_mut().filter(|c| c.was) {
                c.st = St::Starting;
                c.was = false;
                c.since = now;
            }
            for m in s.ms.iter_mut().filter(|m| m.was) {
                m.st = St::Running;
                m.was = false;
                m.fresh_at = now;
                m.since = now;
            }
            for (i, id) in back.into_iter().enumerate() {
                s.after(120 + 160 * i as u64, move |s| {
                    let now = s.time;
                    s.patch(&id, |c| {
                        c.st = St::Running;
                        c.fresh_at = now;
                    });
                });
            }
        });
        self.after(3400, |s| {
            if s.engine == Engine::Ready {
                s.engine = Engine::Running
            }
        });
    }

    pub fn apply_update(&mut self) {
        self.banner = None;
        self.reboot();
        self.after(3500, |s| s.toast_say("Updated to Patina 1.4", "", None));
    }

    // --------------------------------------------------------------- assets

    pub fn ask_asset(&mut self, kind: Kind, id: &str) {
        let boxes: Vec<(String, String)> = self.alive().map(|c| (c.id.clone(), c.grp.clone())).collect();
        match kind {
            Kind::Nets => {
                let Some(a) = self.nets.iter().find(|x| x.n == id) else { return };
                let blocked = match a.attached {
                    Some(n) => n > 0,
                    None => !a.grp.is_empty() && boxes.iter().any(|(_, g)| *g == a.grp),
                };
                self.ask_modal(if blocked { ModalKind::NetBlocked } else { ModalKind::DelNet }, id);
            }
            Kind::Imgs | Kind::Vols => {
                let list = if kind == Kind::Imgs { &self.imgs } else { &self.vols };
                let Some(a) = list.iter().find(|x| x.n == id) else { return };
                let used = boxes.iter().any(|(c, _)| *c == a.used_by);
                let mk = match (kind, used) {
                    (Kind::Imgs, true) => ModalKind::Blocked,
                    (Kind::Imgs, false) => ModalKind::DelImg,
                    (_, true) => ModalKind::VolBlocked,
                    (_, false) => ModalKind::DelVol,
                };
                self.ask_modal(mk, id);
            }
            Kind::Ms => self.ask_modal(ModalKind::DelMachine, id),
        }
    }

    pub fn drop_asset(&mut self, kind: Kind, id: &str, undoable: bool) {
        self.set_gone(kind, id, true);
        if self.live.is_some() {
            self.pending_a.insert(id.to_string());
            self.send(match kind {
                Kind::Imgs => LCmd::RemoveImage(id.into()),
                Kind::Vols => LCmd::RemoveVolume(id.into()),
                Kind::Nets => LCmd::RemoveNetwork(id.into()),
                Kind::Ms => LCmd::MachineDelete(id.into()),
            });
            return;
        }
        self.toast_say(
            format!("Deleted {id}"),
            if undoable { "Undo" } else { "" },
            if undoable { Some(Undo::Asset(kind, id.to_string())) } else { None },
        );
    }

    pub fn create_asset(&mut self, kind: Kind) {
        let word = if kind == Kind::Vols { "volume" } else { "network" };
        let id = self.modal.value.trim().to_string();
        if id.is_empty() {
            self.modal.err = format!("Enter a name for the {word}.");
            return;
        }
        let valid = id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
        if !valid {
            self.modal.err = "Use letters, numbers, dots, dashes and underscores.".into();
            return;
        }
        let taken = if kind == Kind::Vols {
            self.vols.iter().any(|a| a.n == id && !a.gone)
        } else {
            self.nets.iter().any(|a| a.n == id && !a.gone)
        };
        if taken {
            self.modal.err = format!("A {word} named {id} already exists.");
            return;
        }
        if kind == Kind::Vols {
            self.vols.retain(|a| a.n != id);
            let born = self.time;
            self.vols.push(Asset { n: id.clone(), mb: 0.0, used_by: String::new(), age: "just now".into(), fresh: true, gone: false, born });
        } else {
            let subnet = format!("172.{}.0.0/16", 17 + self.nets.len());
            self.nets.retain(|a| a.n != id);
            let born = self.time;
            self.nets.push(Net { n: id.clone(), driver: "bridge".into(), subnet, sys: false, grp: String::new(), fresh: true, gone: false, born, attached: if self.live.is_some() { Some(0) } else { None } });
        }
        self.close_ov();
        if self.live.is_some() {
            self.pending_new.insert(id.clone());
            self.send(if kind == Kind::Vols { LCmd::CreateVolume(id) } else { LCmd::CreateNetwork(id) });
            return;
        }
        self.toast_say(format!("Created {id}"), "", None);
    }

    pub fn spare_images(&self) -> Vec<&Asset> {
        self.imgs.iter().filter(|a| !a.gone && !self.alive().any(|c| c.id == a.used_by)).collect()
    }

    pub fn prune_images(&mut self) {
        let spare: Vec<String> = self.spare_images().iter().map(|a| a.n.clone()).collect();
        let freed: f64 = self.imgs.iter().filter(|a| spare.contains(&a.n)).map(|a| a.mb).sum();
        for a in self.imgs.iter_mut().filter(|a| spare.contains(&a.n)) {
            a.gone = true;
        }
        if self.banner == Some(Banner::Disk) {
            self.banner = None;
        }
        self.close_ov();
        if self.live.is_some() {
            self.pending_a.extend(spare);
            self.send(LCmd::PruneImages);
            return;
        }
        self.toast_say(format!("Freed {}", fmt_size(freed)), "", None);
    }

    pub fn pull_busy(&self) -> bool {
        self.pull.as_ref().is_some_and(|p| !p.err)
    }

    pub fn pull_image(&mut self) {
        if self.engine != Engine::Running || self.pull_busy() {
            return;
        }
        if self.live.is_some() {
            self.ask_modal(ModalKind::Pull, "");
            return;
        }
        if self.pulled {
            return;
        }
        let layers = LAYERS.iter().map(|(id, size)| (id.to_string(), 0.0, size.to_string())).collect();
        self.pull = Some(Pull { image: PULL_IMAGE.into(), layers, done: false, err: false, msg: String::new() });
        self.pull_gen += 1;
        self.pull_turn = 0;
        let gen = self.pull_gen;
        self.after(240, move |s| s.pull_tick(gen));
    }

    fn pull_tick(&mut self, gen: u32) {
        if gen != self.pull_gen {
            return;
        }
        let Some(cur) = self.pull.clone() else { return };
        if cur.err {
            return;
        }
        self.pull_turn += 1;
        if self.preset.scenario == "offline" && self.pull_turn > 3 {
            let msg = "registry-1.docker.io cannot be reached. Check your connection, then try again.".to_string();
            self.pull = Some(Pull { err: true, msg, ..cur });
            return;
        }
        let rate = [0.3, 0.17, 0.085, 0.5];
        let mut layers = cur.layers.clone();
        for (i, l) in layers.iter_mut().enumerate() {
            let r = self.rnd();
            l.1 = (l.1 + rate[i % 4] * (0.7 + r * 0.6)).min(1.0);
        }
        let done = layers.iter().all(|l| l.1 >= 1.0);
        self.pull = Some(Pull { layers, done, err: false, ..cur });
        if done {
            self.after(1100, |s| {
                s.pull = None;
                s.pulled = true;
                let born = s.time;
                s.imgs.push(Asset { n: PULL_IMAGE.into(), mb: 48.0, used_by: String::new(), age: "just now".into(), fresh: true, gone: false, born });
                s.toast_say(format!("Pulled {PULL_IMAGE}"), "", None);
            });
        } else {
            self.after(240, move |s| s.pull_tick(gen));
        }
    }

    /// Live engine: pull the image typed into the Pull dialog.
    pub fn start_pull(&mut self, image: &str) {
        let image = image.trim().to_string();
        if image.is_empty() {
            self.modal.err = "Enter an image, for example nginx:1.27-alpine.".into();
            return;
        }
        if image.contains(char::is_whitespace) {
            self.modal.err = "Image names can't contain spaces.".into();
            return;
        }
        self.close_ov();
        self.go_view(View::Images);
        self.pull = Some(Pull { image: image.clone(), layers: Vec::new(), done: false, err: false, msg: String::new() });
        self.send(LCmd::Pull(image));
    }

    pub fn retry_pull(&mut self) {
        match (&self.live, &self.pull) {
            (Some(_), Some(p)) => {
                let image = p.image.clone();
                self.pull = Some(Pull { image: image.clone(), layers: Vec::new(), done: false, err: false, msg: String::new() });
                self.send(LCmd::Pull(image));
            }
            _ => self.pull_image(),
        }
    }

    // ------------------------------------------------------------- machines

    pub fn flip_machine(&mut self, id: &str) {
        let Some(m) = self.ms.iter().find(|x| x.id == id) else { return };
        if self.engine != Engine::Running {
            return;
        }
        let id = id.to_string();
        if self.live.is_some() {
            match m.st {
                St::Running => {
                    self.pending_m.insert(id.clone(), St::Stopping);
                    self.patch_machine(&id, |m| m.st = St::Stopping);
                    self.send(LCmd::MachineStop(id));
                }
                St::Stopped => {
                    self.pending_m.insert(id.clone(), St::Starting);
                    self.patch_machine(&id, |m| m.st = St::Starting);
                    self.send(LCmd::MachineStart(id));
                }
                _ => {}
            }
            return;
        }
        match m.st {
            St::Running => {
                self.patch_machine(&id, |m| m.st = St::Stopping);
                self.after(800, move |s| s.patch_machine(&id, |m| m.st = St::Stopped));
            }
            St::Stopped => {
                self.patch_machine(&id, |m| m.st = St::Starting);
                self.after(1500, move |s| {
                    let now = s.time;
                    s.patch_machine(&id, |m| {
                        m.st = St::Running;
                        m.fresh_at = now;
                        m.up = "up just now".into();
                    })
                });
            }
            _ => {}
        }
    }

    pub fn make_machine(&mut self) {
        if self.mk.busy {
            return;
        }
        let pick = DISTROS[self.mk.pick.min(DISTROS.len() - 1)];
        let raw = if self.mk.name.trim().is_empty() { pick.0.to_string() } else { self.mk.name.trim().to_lowercase() };
        let mut id: String = raw.chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' { c } else { '-' }).collect();
        if self.ms.iter().any(|m| m.id == id && !m.gone) {
            id.push_str("-2");
        }
        if self.live.is_some() {
            let now = self.time;
            self.ms.retain(|m| m.id != id);
            self.ms.push(Machine {
                id: id.clone(),
                distro: pick.1.to_string(),
                st: St::Starting,
                up: String::new(),
                was: false,
                gone: false,
                added: true,
                fresh_at: -10.0,
                since: now,
                born: now,
                arch: "arm64".into(),
            });
            self.pending_m.insert(id.clone(), St::Starting);
            self.send(LCmd::MachineCreate { distro: pick.0.to_string(), name: id });
            self.go_view(View::Machines);
            self.close_ov();
            return;
        }
        self.mk.busy = true;
        let distro = pick.1.to_string();
        self.after(900, move |s| {
            let now = s.time;
            s.ms.retain(|m| m.id != id);
            s.ms.push(Machine {
                id: id.clone(),
                distro,
                st: St::Starting,
                up: "up just now".into(),
                was: false,
                gone: false,
                added: true,
                fresh_at: -10.0,
                since: now,
                born: now,
                arch: "arm64".into(),
            });
            s.go_view(View::Machines);
            s.close_ov();
            s.after(1700, move |s| {
                let now = s.time;
                s.patch_machine(&id, |m| {
                    m.st = St::Running;
                    m.fresh_at = now;
                })
            });
        });
    }

    // ----------------------------------------------------------- kubernetes

    pub fn flip_kube(&mut self) {
        if self.engine != Engine::Running {
            return;
        }
        match self.kube {
            Kube::Off => {
                self.kube = Kube::Starting;
                self.kstep = 0;
                for p in self.pods.iter_mut() {
                    p.st = St::Starting;
                    p.shown = false;
                }
                self.after(900, |s| s.kstep = 1);
                if self.live.is_some() {
                    self.after(2000, |s| s.kstep = 2);
                    self.kube_busy = true;
                    self.send(LCmd::Kube(true));
                    return;
                }
                for (i, id) in PODS.iter().enumerate() {
                    self.after(1000 + 180 * i as u64, move |s| {
                        let now = s.time;
                        s.patch_pod(id, |p| {
                            p.shown = true;
                            p.shown_at = now;
                        })
                    });
                }
                self.after(2000, |s| s.kstep = 2);
                self.after(2900, |s| {
                    if s.kube == Kube::Starting {
                        s.kube = Kube::On;
                        for (i, id) in PODS.iter().enumerate() {
                            s.after(150 * i as u64, move |s| {
                                let now = s.time;
                                s.patch_pod(id, |p| {
                                    p.st = St::Running;
                                    p.fresh_at = now;
                                })
                            });
                        }
                    }
                });
            }
            Kube::On => {
                self.kube = Kube::Off;
                for p in self.pods.iter_mut() {
                    p.shown = false;
                }
                if self.live.is_some() {
                    self.kube_busy = true;
                    self.send(LCmd::Kube(false));
                }
            }
            Kube::Starting => {}
        }
    }

    pub fn reset_kube(&mut self) {
        self.close_ov();
        if self.kube != Kube::On {
            return;
        }
        self.kube = Kube::Off;
        for p in self.pods.iter_mut() {
            p.shown = false;
        }
        if self.live.is_some() {
            self.kube = Kube::Starting;
            self.kstep = 0;
            self.kube_busy = true;
            self.send(LCmd::KubeReset);
            return;
        }
        self.after(600, |s| s.flip_kube());
    }

    // ------------------------------------------------------------- settings

    pub fn dirty_count(&self) -> usize {
        (self.set.cpus != self.saved.0) as usize + (self.set.mem != self.saved.1) as usize
    }

    pub fn apply_set(&mut self) {
        let on = self.engine == Engine::Running;
        self.saved = (self.set.cpus, self.set.mem);
        if self.live.is_some() && !on {
            self.send(LCmd::ApplyResources { cpus: self.set.cpus, mem_mib: self.set.mem * 1024 });
            self.toast_say("Saved. Applies when the engine starts.", "", None);
            return;
        }
        if self.live.is_some() {
            self.engine = Engine::Booting;
            self.boot = 0;
            self.cold = false;
            self.engine_op = Some(EngineOp::Restart);
            self.op_saw_down = false;
            self.boot_at = self.time;
            self.after(560, |s| s.boot = 1);
            self.after(1120, |s| s.boot = 2);
            self.send(LCmd::ApplyResources { cpus: self.set.cpus, mem_mib: self.set.mem * 1024 });
            self.toast_say(format!("Restarting with {} cores and {} GB", self.set.cpus, self.set.mem), "", None);
            return;
        }
        self.reboot();
        let msg = if on {
            format!("Restarting with {} cores and {} GB", self.set.cpus, self.set.mem)
        } else {
            "Saved. Applies when the engine starts.".to_string()
        };
        self.toast_say(msg, "", None);
    }

    pub fn discard_set(&mut self) {
        self.set.cpus = self.saved.0;
        self.set.mem = self.saved.1;
    }

    pub fn check_upd(&mut self) {
        if self.upd == Upd::Checking {
            return;
        }
        self.upd = Upd::Checking;
        self.after(1500, |s| {
            s.upd = Upd::Current;
            s.upd_at = s.time;
        });
    }

    // ------------------------------------------------------------------ run

    pub fn run_go(&mut self) {
        if self.run.busy {
            return;
        }
        let image = self.run.image.trim().to_string();
        let host = self.run.host.trim().to_string();
        if image.is_empty() {
            self.run.err = "Enter an image, for example nginx:1.27-alpine.".into();
            self.run.perr.clear();
            return;
        }
        if !host.is_empty() {
            let ok = host.chars().all(|c| c.is_ascii_digit()) && host.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n));
            if !ok {
                self.run.perr = "Enter a port between 1 and 65535.".into();
                return;
            }
            let clash = self.alive().find(|c| c.port == host && !matches!(c.st, St::Stopped | St::Error)).map(|c| c.id.clone());
            if let Some(owner) = clash {
                self.run.perr = format!("Port {host} is already used by {owner}. Choose another port.");
                return;
            }
        }
        let raw = if self.run.name.trim().is_empty() {
            let last = image.rsplit('/').next().unwrap_or(&image);
            last.split(':').next().unwrap_or(last).to_string()
        } else {
            self.run.name.trim().to_string()
        };
        let id: String = raw
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-') { c } else { '-' })
            .collect();
        if self.alive().any(|c| c.id == id) {
            self.run.perr = format!("A container named {id} already exists. Give this one another name.");
            return;
        }
        self.run.err.clear();
        self.run.perr.clear();
        self.run.busy = true;
        if self.live.is_some() {
            let port = self.run.port.trim().to_string();
            self.send(LCmd::Run { image, name: id, host, port });
            return;
        }
        self.after(1500, move |s| {
            let hex: String = (0..12).map(|_| b"0123456789abcdef"[(s.rnd() * 16.0) as usize % 16] as char).collect();
            let mut c = s.mk_box(&id, &image, &host, "standalone", St::Starting, 0.5, 36.0, &hex, "just now");
            c.added = true;
            c.since = s.time;
            c.born = s.time;
            s.cs.retain(|x| x.id != id);
            s.cs.push(c);
            s.sel = id.clone();
            s.panel_at = s.time;
            s.go_view(View::Containers);
            s.filter = Filter::All;
            s.q.clear();
            let clock = s.clock;
            s.add_log(&id, clock, Some((format!("pulled {image}"), 0)));
            s.close_ov();
            s.toast_say(format!("Created {id}"), "", None);
            s.after(1400, move |s| {
                let now = s.time;
                s.patch(&id, |c| {
                    c.st = St::Running;
                    c.fresh_at = now;
                });
                s.log_note(&id, "started in 1.4 s", 1);
            });
        });
    }

    // -------------------------------------------------------------- palette

    pub fn pal_commands(&self) -> Vec<Cmd> {
        let on = self.engine == Engine::Running;
        let mut list = vec![
            Cmd { label: "Run an image".into(), hint: "Action", run: Box::new(|s: &mut State| s.open_run("")) },
            Cmd {
                label: "Pull an image".into(),
                hint: "Action",
                run: Box::new(|s: &mut State| {
                    s.go_view(View::Images);
                    s.pull_image();
                }),
            },
            Cmd {
                label: if on { "Restart the engine" } else { "Start the engine" }.into(),
                hint: "Engine",
                run: Box::new(move |s: &mut State| if on { s.reboot() } else { s.power_flip() }),
            },
        ];
        for c in self.alive() {
            let id = c.id.clone();
            list.push(Cmd {
                label: format!("{} {}", if c.st == St::Running { "Stop" } else { "Start" }, c.id),
                hint: "Container",
                run: Box::new(move |s: &mut State| {
                    s.go_view(View::Containers);
                    s.toggle(&id);
                }),
            });
        }
        for v in View::ALL {
            list.push(Cmd { label: format!("Go to {}", v.label()), hint: "Page", run: Box::new(move |s: &mut State| s.go_view(v)) });
        }
        list
    }

    pub fn pal_filtered(&self) -> Vec<Cmd> {
        let want = self.pal.q.trim().to_lowercase();
        self.pal_commands().into_iter().filter(|c| want.is_empty() || c.label.to_lowercase().contains(&want)).take(8).collect()
    }

    pub fn pal_run(&mut self, cmd: Cmd) {
        self.close_ov();
        self.after(190, cmd.run);
    }

    // ---------------------------------------------------------------- modal

    pub fn modal_spec(&self) -> ModalSpec {
        let m = &self.modal;
        let t = m.target.clone();
        let base = ModalSpec {
            title: String::new(),
            body: String::new(),
            field: false,
            label: "",
            ph: "",
            yes: "OK".into(),
            no: "Cancel",
            danger: false,
            go: Box::new(|s: &mut State| s.close_ov()),
        };
        let del = |body: &str, go: Job| ModalSpec {
            title: format!("Delete {t}?"),
            body: body.into(),
            yes: "Delete".into(),
            danger: true,
            go,
            field: false,
            label: "",
            ph: "",
            no: "Cancel",
        };
        let reveal = |id: String| -> Job {
            Box::new(move |s: &mut State| {
                s.close_ov();
                s.go_view(View::Containers);
                s.pick(&id);
            })
        };
        let n_box = self.alive().count();
        let _ = n_box;
        let Some(kind) = m.kind else { return base };
        let tt = t.clone();
        match kind {
            ModalKind::Delete => del(
                "The container and its writable layer are removed. Its image and volumes stay.",
                Box::new(move |s| {
                    s.close_ov();
                    s.drop_box(&tt);
                }),
            ),
            ModalKind::DelImg => del(
                "The image is removed from this Mac. You can pull it again later.",
                Box::new(move |s| {
                    s.close_ov();
                    s.drop_asset(Kind::Imgs, &tt, true);
                }),
            ),
            ModalKind::DelVol => del(
                "Everything stored in this volume is lost. This cannot be undone.",
                Box::new(move |s| {
                    s.close_ov();
                    s.drop_asset(Kind::Vols, &tt, false);
                }),
            ),
            ModalKind::DelNet => del(
                "Containers can no longer join this network.",
                Box::new(move |s| {
                    s.close_ov();
                    s.drop_asset(Kind::Nets, &tt, true);
                }),
            ),
            ModalKind::DelMachine => del(
                "The machine and everything on its disk are removed. This cannot be undone.",
                Box::new(move |s| {
                    s.close_ov();
                    s.drop_asset(Kind::Ms, &tt, false);
                }),
            ),
            ModalKind::Blocked => {
                let used = self.imgs.iter().find(|a| a.n == t).map(|a| a.used_by.clone()).unwrap_or_default();
                ModalSpec {
                    title: format!("{t} is in use"),
                    body: format!("{used} was created from this image. Delete that container first, then delete the image."),
                    no: "Close",
                    yes: format!("Show {used}"),
                    go: reveal(used),
                    ..base
                }
            }
            ModalKind::VolBlocked => {
                let used = self.vols.iter().find(|a| a.n == t).map(|a| a.used_by.clone()).unwrap_or_default();
                ModalSpec {
                    title: format!("{t} is in use"),
                    body: format!("{used} has this volume mounted. Delete that container first, then delete the volume."),
                    no: "Close",
                    yes: format!("Show {used}"),
                    go: reveal(used),
                    ..base
                }
            }
            ModalKind::NetBlocked => {
                let grp = self.nets.iter().find(|a| a.n == t).map(|a| a.grp.clone()).unwrap_or_default();
                let n = self.alive().filter(|c| c.grp == grp).count();
                ModalSpec {
                    title: format!("{t} has containers attached"),
                    body: format!("{} on this network. Delete them first, then delete the network.", fmt_count(n, "container")),
                    no: "Close",
                    yes: "Show containers".into(),
                    go: Box::new(|s| {
                        s.close_ov();
                        s.go_view(View::Containers);
                    }),
                    ..base
                }
            }
            ModalKind::Prune => {
                let spare = self.spare_images();
                let mb: f64 = spare.iter().map(|a| a.mb).sum();
                ModalSpec {
                    title: format!("Remove {}?", fmt_count(spare.len(), "unused image")),
                    body: format!("This frees {}. Images that a container uses are kept.", fmt_size(mb)),
                    yes: "Remove".into(),
                    danger: true,
                    go: Box::new(|s| s.prune_images()),
                    ..base
                }
            }
            ModalKind::Volume => ModalSpec {
                title: "Create a volume".into(),
                body: "A volume keeps data when its container is deleted.".into(),
                field: true,
                label: "Name",
                ph: "my_data",
                yes: "Create".into(),
                go: Box::new(|s| s.create_asset(Kind::Vols)),
                ..base
            },
            ModalKind::Network => ModalSpec {
                title: "Create a network".into(),
                body: "Containers on the same network reach each other by name.".into(),
                field: true,
                label: "Name",
                ph: "my_network",
                yes: "Create".into(),
                go: Box::new(|s| s.create_asset(Kind::Nets)),
                ..base
            },
            ModalKind::Pull => ModalSpec {
                title: "Pull an image".into(),
                body: "Downloads an image from its registry, such as Docker Hub.".into(),
                field: true,
                label: "Image",
                ph: "nginx:1.27-alpine",
                yes: "Pull".into(),
                go: Box::new(|s| {
                    let v = s.modal.value.clone();
                    s.start_pull(&v)
                }),
                ..base
            },
            ModalKind::Reset => ModalSpec {
                title: "Reset the Kubernetes cluster?".into(),
                body: "All workloads, services and stored cluster data are deleted. Your images are kept.".into(),
                yes: "Reset".into(),
                danger: true,
                go: Box::new(|s| s.reset_kube()),
                ..base
            },
        }
    }

    pub fn boot_log() -> &'static str {
        "14:02:09 vmm     creating VM with 4 CPUs, 8192 MB\n14:02:09 vmm     error: not enough free memory (1.1 GB free)\n14:02:09 engine  start failed after 0.4 s"
    }

    // ----------------------------------------------------------- live engine

    /// A state wired to the real engine. It starts empty and fills in as
    /// the backend reports.
    pub fn new_live(handle: live::Handle, nav: Preset, now: f64) -> State {
        let mut s = State::new(Preset { scenario: "empty".into(), view: nav.view, pane: nav.pane, ..Default::default() }, now);
        s.nets.clear();
        s.pods.clear();
        s.sel.clear();
        s.engine = Engine::Booting;
        s.cold = true;
        s.live = Some(handle);
        s.installed = crate::engine::setup::installed();
        let cfg = crate::engine::config::load();
        s.saved = (cfg.cpus, cfg.memory_gb);
        s.set.cpus = cfg.cpus;
        s.set.mem = cfg.memory_gb;
        if s.installed && cfg.autostart && crate::engine::control::status().is_none() {
            s.cold_boot();
        }
        s
    }

    pub fn is_live(&self) -> bool {
        self.live.is_some()
    }

    pub fn refresh(&self) {
        self.send(LCmd::Refresh);
    }

    pub fn open_terminal(&self, id: &str) {
        self.send(LCmd::OpenTerminal(id.to_string()));
    }

    /// Change an engine setting (saved to `~/.patina/config.json`).
    pub fn set_cfg(&mut self, key: &str, on: bool, needs_restart: bool) {
        self.cfg.insert(key.to_string(), on.to_string());
        self.send(LCmd::Config(key.to_string(), on.to_string()));
        if needs_restart {
            self.toast_say("Saved. Applies after the engine restarts.", "", None);
        }
    }

    pub fn cfg_on(&self, key: &str) -> bool {
        self.cfg.get(key).is_some_and(|v| v == "true")
    }

    pub fn apply(&mut self, ev: live::Ev) {
        let now = self.time;
        match ev {
            live::Ev::Up(info) => {
                let disk = self.info.disk_gb;
                self.info = info;
                if self.info.disk_gb == 0.0 {
                    self.info.disk_gb = disk;
                }
                self.installed = true;
                self.engine_state = "running".into();
                self.endpoint = self.info.endpoint.clone();
                match self.engine_op {
                    Some(EngineOp::Stop) => {}
                    Some(EngineOp::Restart) if !self.op_saw_down => {}
                    op => {
                        if !matches!(self.engine, Engine::Running | Engine::Ready) {
                            self.cold = false;
                            if op.is_some() {
                                self.ready_secs = now - self.boot_at;
                                self.engine = Engine::Ready;
                                self.after(1700, |s| {
                                    if s.engine == Engine::Ready {
                                        s.engine = Engine::Running
                                    }
                                });
                            } else {
                                self.engine = Engine::Running;
                            }
                        }
                    }
                }
            }
            live::Ev::Down { installed, status } => {
                self.installed = installed;
                let state = status.as_ref().map(|s| s.state.clone()).unwrap_or_default();
                if let Some(s) = &status {
                    self.engine_msg = s.message.clone();
                }
                self.engine_state = state.clone();
                match state.as_str() {
                    "starting" => self.boot = 0,
                    "booting" => self.boot = 1,
                    "installing" => self.boot = 2,
                    _ => {}
                }
                if let Some(op) = self.engine_op {
                    if op == EngineOp::Restart && state != "running" {
                        self.op_saw_down = true;
                    }
                    return;
                }
                match state.as_str() {
                    "starting" | "booting" | "installing" => {
                        if self.engine != Engine::Booting {
                            self.engine = Engine::Booting;
                            self.cold = true;
                        }
                    }
                    "error" => {
                        self.engine = Engine::Failed;
                        self.cold = false;
                        self.engine_log = crate::engine::control::log_tail(12);
                    }
                    "stopping" => {
                        self.engine = Engine::Off;
                        self.cold = false;
                    }
                    _ => {
                        if self.engine != Engine::Failed {
                            self.engine = Engine::Off;
                            self.cold = false;
                        }
                    }
                }
            }
            live::Ev::Containers(list) => self.merge_boxes(list),
            live::Ev::Disk(gb) => self.info.disk_gb = gb,
            live::Ev::Stats { id, cpu, mem_mb, limit_mb } => {
                if let Some(c) = self.cs.iter_mut().find(|c| c.id == id && c.st == St::Running) {
                    c.cpu = cpu;
                    c.mem = mem_mb.round();
                    if limit_mb > 0.0 {
                        c.mem_limit = limit_mb;
                    }
                    c.bars.rotate_left(1);
                    c.bars[11] = lv(cpu);
                }
            }
            live::Ev::Inspect { id, why, restart, hostname, loops } => {
                if let Some(c) = self.cs.iter_mut().find(|c| c.id == id) {
                    c.why = why;
                    c.restart = restart;
                    c.hostname = hostname;
                    if loops > 0 {
                        c.loops = loops;
                    }
                }
            }
            live::Ev::Logs { id, lines, reset } => {
                let lines: Vec<LogLine> = lines.into_iter().map(|(t, m, k)| LogLine { t, m, k }).collect();
                if reset {
                    self.logs.insert(id, lines);
                } else {
                    let list = self.logs.entry(id.clone()).or_default();
                    list.extend(lines);
                    let n = list.len();
                    if n > 10 {
                        list.drain(0..n - 10);
                    }
                    if id == self.sel {
                        self.log_at = now;
                    }
                }
            }
            live::Ev::Assets { imgs, vols, nets } => {
                let first = !self.seen_assets;
                self.seen_assets = true;
                let (pa, pn) = (self.pending_a.clone(), self.pending_new.clone());
                let merge = |old: &[Asset], new: Vec<Asset>| -> Vec<Asset> {
                    let mut out: Vec<Asset> = new
                        .into_iter()
                        .map(|mut a| {
                            match old.iter().find(|o| o.n == a.n) {
                                Some(o) => {
                                    a.fresh = o.fresh;
                                    a.born = o.born;
                                }
                                None if !first => {
                                    a.fresh = true;
                                    a.born = now;
                                }
                                None => {}
                            }
                            a.gone = pa.contains(&a.n);
                            a
                        })
                        .collect();
                    let keep: Vec<Asset> = old.iter().filter(|o| pn.contains(&o.n) && !out.iter().any(|a| a.n == o.n)).cloned().collect();
                    out.extend(keep);
                    out
                };
                self.imgs = merge(&self.imgs, imgs);
                self.vols = merge(&self.vols, vols);
                let mut out: Vec<Net> = nets
                    .into_iter()
                    .map(|mut a| {
                        match self.nets.iter().find(|o| o.n == a.n) {
                            Some(o) => {
                                a.fresh = o.fresh;
                                a.born = o.born;
                            }
                            None if !first => {
                                a.fresh = true;
                                a.born = now;
                            }
                            None => {}
                        }
                        a.gone = pa.contains(&a.n);
                        a
                    })
                    .collect();
                let keep: Vec<Net> = self.nets.iter().filter(|o| pn.contains(&o.n) && !out.iter().any(|a| a.n == o.n)).cloned().collect();
                out.extend(keep);
                self.nets = out;
            }
            live::Ev::Machines(list) => {
                self.machines_ok = list.is_some();
                let Some(list) = list else {
                    self.ms.clear();
                    return;
                };
                let mut out = Vec::new();
                for m in list {
                    let prev = self.ms.iter().find(|x| x.id == m.id).cloned();
                    let st = self.pending_m.get(&m.id).copied().unwrap_or(m.st);
                    let mut x = prev.clone().unwrap_or(Machine {
                        id: m.id.clone(),
                        distro: m.distro.clone(),
                        st,
                        up: String::new(),
                        was: false,
                        gone: false,
                        added: self.seen_boxes,
                        fresh_at: -10.0,
                        since: now,
                        born: if self.seen_boxes { now } else { -10.0 },
                        arch: m.arch.clone(),
                    });
                    if x.st != st {
                        x.since = now;
                        if st == St::Running {
                            x.fresh_at = now;
                        }
                    }
                    x.st = st;
                    x.distro = m.distro;
                    x.arch = m.arch;
                    x.gone = self.pending_a.contains(&x.id);
                    out.push(x);
                }
                let keep: Vec<Machine> = self.ms.iter().filter(|m| self.pending_m.contains_key(&m.id) && !out.iter().any(|x| x.id == m.id)).cloned().collect();
                out.extend(keep);
                self.ms = out;
            }
            live::Ev::Kube { enabled, reachable, pods } => {
                if !self.kube_busy {
                    let all_running = !pods.is_empty() && pods.iter().all(|p| p.running);
                    self.kube = if !enabled {
                        Kube::Off
                    } else if reachable && all_running {
                        Kube::On
                    } else {
                        Kube::Starting
                    };
                }
                if self.kube == Kube::Off {
                    for p in self.pods.iter_mut() {
                        p.shown = false;
                    }
                    return;
                }
                let mut out = Vec::new();
                for p in pods {
                    let prev = self.pods.iter().find(|x| x.id == p.id);
                    let st = if p.running { St::Running } else { St::Starting };
                    let fresh_at = match prev {
                        Some(x) if x.st != St::Running && st == St::Running => now,
                        Some(x) => x.fresh_at,
                        None => -10.0,
                    };
                    let shown_at = match prev {
                        Some(x) if x.shown => x.shown_at,
                        _ => now,
                    };
                    out.push(Pod { id: p.id, st, fresh_at, shown: true, shown_at, ready: p.ready, phase: p.phase });
                }
                self.pods = out;
            }
            live::Ev::Config(map) => {
                let clean = self.set.cpus == self.saved.0 && self.set.mem == self.saved.1;
                let cpus = map.get("cpu").and_then(|v| v.parse::<u32>().ok());
                let mem = map.get("memory_mib").and_then(|v| v.parse::<u32>().ok()).map(|m| (m + 512) / 1024);
                if let (Some(c), Some(m)) = (cpus, mem) {
                    self.saved = (c, m);
                    if clean || self.cfg.is_empty() {
                        self.set.cpus = c;
                        self.set.mem = m;
                    }
                }
                self.cfg = map;
            }
            live::Ev::Pull { image, layers, done, err } => {
                let layers = layers.into_iter().map(|(id, f)| (id, f, String::new())).collect();
                self.pull = Some(Pull { image: image.clone(), layers, done, err: err.is_some(), msg: err.unwrap_or_default() });
                if done {
                    self.after(1100, move |s| {
                        if s.pull.as_ref().is_some_and(|p| p.done) {
                            s.pull = None;
                        }
                        s.toast_say(format!("Pulled {image}"), "", None);
                    });
                }
            }
            live::Ev::Done { key, ok, msg } => {
                let (kind, arg) = key.split_once(':').unwrap_or((key.as_str(), ""));
                let arg = arg.to_string();
                match kind {
                    "box" => {
                        self.pending.remove(&arg);
                        if !ok {
                            self.toast_say(msg, "", None);
                        }
                    }
                    "rm" => {
                        self.pending.remove(&arg);
                        if !ok {
                            self.patch(&arg, |c| c.gone = false);
                        }
                        self.toast_say(msg, "", None);
                    }
                    "run" => {
                        self.run.busy = false;
                        if ok {
                            self.close_ov();
                            self.sel = msg.clone();
                            self.panel_at = now;
                            self.go_view(View::Containers);
                            self.filter = Filter::All;
                            self.q.clear();
                            self.toast_say(format!("Created {msg}"), "", None);
                        } else {
                            self.run.perr = msg;
                        }
                    }
                    "pull" => {
                        if !ok {
                            let image = self.pull.as_ref().map(|p| p.image.clone()).unwrap_or_default();
                            self.pull = Some(Pull { image, layers: Vec::new(), done: false, err: true, msg });
                        }
                    }
                    "engine" => {
                        let op = self.engine_op.take();
                        self.op_saw_down = false;
                        if !ok {
                            self.engine = Engine::Failed;
                            self.cold = false;
                            let tail = crate::engine::control::log_tail(12);
                            self.engine_log = if tail.is_empty() { msg } else { format!("{msg}\n\n{tail}") };
                        } else if op == Some(EngineOp::Stop) {
                            self.engine = Engine::Off;
                        }
                    }
                    "asset" => {
                        self.pending_a.remove(&arg);
                        if !ok {
                            for k in [Kind::Imgs, Kind::Vols, Kind::Nets, Kind::Ms] {
                                self.set_gone(k, &arg, false);
                            }
                        }
                        self.toast_say(msg, "", None);
                    }
                    "create" => {
                        self.pending_new.remove(&arg);
                        if !ok {
                            self.vols.retain(|a| a.n != arg || !a.fresh);
                            self.nets.retain(|a| a.n != arg || !a.fresh);
                        }
                        self.toast_say(msg, "", None);
                    }
                    "prune" => {
                        if !ok {
                            self.pending_a.clear();
                        }
                        self.toast_say(msg, "", None);
                    }
                    "machine" => {
                        self.pending_m.remove(&arg);
                        if !ok {
                            self.toast_say(msg, "", None);
                        }
                    }
                    "mk" => {
                        self.pending_m.remove(&arg);
                        if !ok {
                            self.ms.retain(|m| m.id != arg);
                        }
                        self.toast_say(msg, "", None);
                    }
                    "kube" => {
                        self.kube_busy = false;
                        if !ok || !msg.is_empty() {
                            self.toast_say(msg, "", None);
                        }
                    }
                    _ => {
                        if !ok || !msg.is_empty() {
                            self.toast_say(msg, "", None);
                        }
                    }
                }
            }
        }
    }

    fn merge_boxes(&mut self, list: Vec<live::BoxSnap>) {
        let now = self.time;
        let first = !self.seen_boxes;
        self.seen_boxes = true;
        let mut out = Vec::with_capacity(list.len());
        for b in list {
            let prev = self.cs.iter().find(|c| c.id == b.id).cloned();
            let is_new = prev.is_none();
            let mut c = match prev {
                Some(p) => p,
                None => {
                    let mut c = self.mk_box(&b.id, &b.image, &b.port, &b.grp, b.st, 0.0, 0.0, &b.hex, &b.up);
                    c.restart.clear();
                    if !first {
                        c.added = true;
                        c.born = now;
                    }
                    c
                }
            };
            c.image = b.image;
            c.port = b.port;
            c.ports_text = b.ports_text;
            c.grp = b.grp;
            c.net = b.net;
            c.hex = b.hex;
            c.up = b.up;
            c.bad = b.bad;
            let pending = self.pending.get(&c.id).copied();
            let (st, gone) = match pending {
                Some(Pending::St(p)) => (p, false),
                Some(Pending::Gone) => (c.st, true),
                None => {
                    c.tag = b.tag;
                    c.loops = if b.loops > 0 { c.loops.max(1) } else { 0 };
                    (b.st, false)
                }
            };
            if st != c.st {
                c.since = now;
                if st == St::Running && !is_new {
                    c.fresh_at = now;
                }
                if !matches!(st, St::Error | St::Starting) && !c.bad {
                    c.why.clear();
                }
            }
            c.st = st;
            c.gone = gone;
            c.was = false;
            if st != St::Running {
                c.cpu = 0.0;
                if st != St::Paused && !c.bars.iter().all(|&v| v == IDLE_BAR) {
                    c.bars.rotate_left(1);
                    c.bars[11] = IDLE_BAR;
                }
            }
            out.push(c);
        }
        self.cs = out;
        if !self.alive().any(|c| c.id == self.sel) {
            let first = self.alive().next().map(|c| c.id.clone());
            if let Some(f) = first {
                self.sel = f;
                self.panel_at = now;
            }
        }
    }

    /// Memory used by the engine, in GB.
    pub fn used_gb(&self) -> f64 {
        if self.live.is_some() {
            return self.alive().filter(|c| matches!(c.st, St::Running | St::Paused)).map(|c| c.mem).sum::<f64>() / 1024.0;
        }
        0.9 + self.alive().filter(|c| c.st == St::Running).map(|c| c.mem).sum::<f64>() / 1000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(view: &str, scenario: &str) -> State {
        State::new(Preset { view: view.into(), scenario: scenario.into(), ..Default::default() }, 0.0)
    }

    /// Advance simulated time in small steps, like frames would.
    fn run_for(s: &mut State, secs: f64) {
        let end = s.time + secs;
        let mut t = s.time;
        while t < end {
            t += 0.05;
            s.tick(t);
        }
    }

    fn st(s: &State, id: &str) -> St {
        s.cs.iter().find(|c| c.id == id).unwrap().st
    }

    #[test]
    fn default_state_matches_the_design() {
        let s = state("", "");
        assert_eq!(s.alive().count(), 7);
        assert_eq!(s.alive().filter(|c| c.st == St::Running).count(), 5);
        assert_eq!(s.imgs.len(), 10);
        assert_eq!(s.vols.len(), 5);
        assert_eq!(s.nets.len(), 4);
        assert_eq!(s.ms.len(), 2);
        assert_eq!(s.view, View::Containers);
        assert_eq!(s.sel, "web");
    }

    #[test]
    fn empty_scenario_has_nothing() {
        let s = state("", "empty");
        assert_eq!(s.alive().count(), 0);
        assert!(s.imgs.is_empty() && s.vols.is_empty() && s.ms.is_empty());
    }

    #[test]
    fn stop_then_start_a_container() {
        let mut s = state("", "");
        s.toggle("web");
        assert_eq!(st(&s, "web"), St::Stopping);
        run_for(&mut s, 0.9);
        assert_eq!(st(&s, "web"), St::Stopped);
        s.toggle("web");
        assert_eq!(st(&s, "web"), St::Starting);
        run_for(&mut s, 1.4);
        assert_eq!(st(&s, "web"), St::Running);
        assert!(s.logs["web"].iter().any(|l| l.m == "started in 1.3 s"));
    }

    #[test]
    fn run_image_validates_then_creates() {
        let mut s = state("", "");
        s.open_run("");
        s.run_go();
        assert!(s.run.err.starts_with("Enter an image"));
        s.run.image = "nginx:1.27-alpine".into();
        s.run.host = "3000".into();
        s.run_go();
        assert_eq!(s.run.perr, "Port 3000 is already used by web. Choose another port.");
        s.run.host = "99999".into();
        s.run_go();
        assert_eq!(s.run.perr, "Enter a port between 1 and 65535.");
        s.run.host = "8088".into();
        s.run_go();
        assert!(s.run.busy);
        run_for(&mut s, 1.6);
        assert_eq!(st(&s, "nginx"), St::Starting);
        assert_eq!(s.sel, "nginx");
        run_for(&mut s, 1.5);
        assert_eq!(st(&s, "nginx"), St::Running);
        // same name again is refused
        s.open_run("nginx:1.27-alpine");
        s.run_go();
        assert!(s.run.perr.contains("already exists"));
    }

    #[test]
    fn delete_and_undo() {
        let mut s = state("", "");
        s.drop_box("worker");
        assert!(s.cs.iter().find(|c| c.id == "worker").unwrap().gone);
        assert_eq!(s.toast.as_ref().unwrap().act, "Undo");
        s.undo_last();
        assert!(!s.cs.iter().find(|c| c.id == "worker").unwrap().gone);
        assert!(s.toast.is_none());
    }

    #[test]
    fn create_volume_checks_names() {
        let mut s = state("volumes", "");
        s.ask_modal(ModalKind::Volume, "");
        s.create_asset(Kind::Vols);
        assert_eq!(s.modal.err, "Enter a name for the volume.");
        s.modal.value = "bad name!".into();
        s.create_asset(Kind::Vols);
        assert_eq!(s.modal.err, "Use letters, numbers, dots, dashes and underscores.");
        s.modal.value = "storefront_pgdata".into();
        s.create_asset(Kind::Vols);
        assert_eq!(s.modal.err, "A volume named storefront_pgdata already exists.");
        s.modal.value = "my_data".into();
        s.create_asset(Kind::Vols);
        assert!(s.vols.iter().any(|v| v.n == "my_data"));
    }

    #[test]
    fn images_in_use_cannot_be_deleted() {
        let mut s = state("images", "");
        s.ask_asset(Kind::Imgs, "node:22-alpine");
        assert_eq!(s.modal.kind, Some(ModalKind::Blocked));
        s.ask_asset(Kind::Imgs, "alpine:3.20");
        assert_eq!(s.modal.kind, Some(ModalKind::DelImg));
    }

    #[test]
    fn prune_keeps_images_in_use() {
        let mut s = state("images", "");
        let spare: Vec<String> = s.spare_images().iter().map(|a| a.n.clone()).collect();
        assert_eq!(spare, ["adminer:4", "ubuntu:24.04", "alpine:3.20"]);
        s.prune_images();
        assert_eq!(s.imgs.iter().filter(|a| !a.gone).count(), 7);
        assert_eq!(s.toast.as_ref().unwrap().msg, "Freed 204 MB");
    }

    #[test]
    fn engine_stop_and_cold_boot_resume_containers() {
        let mut s = state("", "");
        s.power_flip();
        assert_eq!(s.engine, Engine::Off);
        assert_eq!(s.alive().filter(|c| c.st == St::Running).count(), 0);
        s.power_flip();
        assert_eq!(s.engine, Engine::Booting);
        run_for(&mut s, 3.5);
        assert_eq!(s.engine, Engine::Running);
        assert_eq!(s.alive().filter(|c| c.st == St::Running).count(), 5);
    }

    #[test]
    fn pull_completes_or_fails_offline() {
        let mut s = state("images", "");
        s.pull_image();
        run_for(&mut s, 5.0);
        assert!(s.pulled);
        assert!(s.imgs.iter().any(|a| a.n == PULL_IMAGE));

        let mut s = state("images", "offline");
        s.pull_image();
        run_for(&mut s, 2.0);
        assert!(s.pull.as_ref().unwrap().err);
    }

    #[test]
    fn kubernetes_boots_its_pods() {
        let mut s = state("kubernetes", "");
        s.flip_kube();
        assert_eq!(s.kube, Kube::Starting);
        run_for(&mut s, 3.8);
        assert_eq!(s.kube, Kube::On);
        assert!(s.pods.iter().all(|p| p.shown && p.st == St::Running));
    }

    #[test]
    fn palette_filters_commands() {
        let mut s = state("", "");
        s.pal.q = "go to".into();
        assert_eq!(s.pal_filtered().len(), 7);
        s.pal.q = "backup".into();
        assert!(s.pal_filtered().is_empty());
    }

    #[test]
    fn settings_apply_restarts_the_engine() {
        let mut s = state("settings", "");
        s.set.cpus = 6;
        assert_eq!(s.dirty_count(), 1);
        s.apply_set();
        assert_eq!(s.saved, (6, 8));
        assert_eq!(s.engine, Engine::Booting);
        run_for(&mut s, 3.5);
        assert_eq!(s.engine, Engine::Running);
    }
}
