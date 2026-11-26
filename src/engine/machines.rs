//! Linux machines: long-lived Linux systems that share the engine's kernel.
//!
//! A machine is a privileged Docker container that boots its distro's own
//! init (systemd, or OpenRC on Alpine), so services, `systemctl` and
//! `sudo` work as they do on a server. Its files live in the container's
//! writable layer and last until the machine is deleted. It has a user
//! named after yours on the Mac, with the same uid and passwordless sudo,
//! and the Mac's home folder at the same path, as containers do.
//!
//! The first machine of a distro builds a base image, `patina.local/<distro>`:
//! a short-lived container from the distro's official image installs init
//! and sudo and adds the user, and is committed with init as the entrypoint.
//! Later machines of that distro start from it in a second. Arch Linux has
//! no arm64 image on Docker Hub, so its root filesystem comes from
//! images.linuxcontainers.org (the images Incus and LXD use), checked
//! against the SHA-256 published with it.
//!
//! Each machine also has a fixed address on the network macOS gives the VM
//! (192.168.64.0/24 by default), through an ipvlan network that shares the
//! VM's MAC address, so the Mac reaches it on any port. Its default route
//! stays on Docker's bridge, as for containers. The engine names it
//! `<name>.patina.local` on this Mac (see `mdns`).

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use bollard::models::{
    ContainerCreateBody, ContainerSummary, EndpointIpamConfig, EndpointSettings, HostConfig, HostConfigCgroupnsModeEnum, Ipam,
    IpamConfig, NetworkConnectRequest, NetworkCreateRequest, RestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CommitContainerOptionsBuilder, CreateContainerOptionsBuilder, CreateImageOptionsBuilder, InspectContainerOptionsBuilder,
    ListContainersOptionsBuilder, RemoveContainerOptionsBuilder, RemoveImageOptionsBuilder,
};
use bollard::Docker;
use futures_util::StreamExt;

use super::{kube, paths};

/// The machine's name, on its container.
pub const LABEL: &str = "dev.patina.machine";
/// The distro and version, as the card shows them ("Ubuntu 24.04 LTS").
const DISTRO_LABEL: &str = "dev.patina.distro";
const ARCH_LABEL: &str = "dev.patina.arch";
/// On base images: which [`BASE_REV`] built them.
const BASE_LABEL: &str = "dev.patina.base";
/// On the container that builds a base image. Base images (and so machines)
/// inherit it, because a commit can't drop labels; it only hides them.
pub const BUILD_LABEL: &str = "dev.patina.machine-build";
/// Bumped when the base images change, so they're rebuilt for new machines.
/// 2: services start when they're installed (no policy-rc.d).
const BASE_REV: &str = "2";
/// The ipvlan network that gives machines their addresses.
pub const LAN: &str = "patina-machines";
/// Machines are `<name>.patina.local` on this Mac.
pub const DOMAIN: &str = "patina.local";
/// Where macOS records the addresses it lends to VMs.
const LEASES: &str = "/var/db/dhcpd_leases";

pub struct Distro {
    /// The key the UI uses (`sim::DISTROS`).
    pub key: &'static str,
    title: &'static str,
    /// The official image, or `None` for images.linuxcontainers.org.
    hub: Option<&'static str>,
    /// The base image Patina builds.
    base: &'static str,
    /// The signal that shuts its init down cleanly.
    stop: &'static str,
}

pub const DISTROS: [Distro; 5] = [
    Distro { key: "ubuntu", title: "Ubuntu", hub: Some("ubuntu:24.04"), base: "patina.local/ubuntu:24.04", stop: "SIGRTMIN+3" },
    Distro { key: "debian", title: "Debian", hub: Some("debian:13"), base: "patina.local/debian:13", stop: "SIGRTMIN+3" },
    Distro { key: "fedora", title: "Fedora", hub: Some("fedora:latest"), base: "patina.local/fedora:latest", stop: "SIGRTMIN+3" },
    Distro { key: "arch", title: "Arch Linux", hub: None, base: "patina.local/archlinux:latest", stop: "SIGRTMIN+3" },
    // Busybox init powers off on SIGUSR2, after OpenRC's shutdown.
    Distro { key: "alpine", title: "Alpine", hub: Some("alpine:latest"), base: "patina.local/alpine:latest", stop: "SIGUSR2" },
];

/// Setting up a base downloads packages; past this, the mirror is stuck.
const SETUP_LIMIT: Duration = Duration::from_secs(20 * 60);

/// Arch's root filesystem, imported here before it's set up.
const ARCH_ROOTFS: &str = "patina.local/archlinux-rootfs:latest";

/// Only one base image is built at a time, so two new machines of the same
/// distro share one.
static BUILDING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Addresses are handed out one at a time.
static ADDRESSING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn label<'a>(c: &'a ContainerSummary, key: &str) -> Option<&'a str> {
    c.labels.as_ref()?.get(key).map(String::as_str)
}

/// What a machine's container says about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub name: String,
    pub distro: String,
    pub arch: String,
    /// Its address on the Mac's VM network; empty until it has one.
    pub ip: String,
}

pub fn info(c: &ContainerSummary) -> Option<Info> {
    let name = label(c, LABEL)?;
    Some(Info {
        name: name.to_string(),
        distro: label(c, DISTRO_LABEL).unwrap_or("Linux").to_string(),
        arch: label(c, ARCH_LABEL).unwrap_or("arm64").to_string(),
        ip: c.network_settings.as_ref().and_then(|n| n.networks.as_ref()).and_then(|n| n.get(LAN)).map(address).unwrap_or_default(),
    })
}

/// An endpoint's address: the one in use, or the one it's set to when the
/// container is stopped.
fn address(e: &EndpointSettings) -> String {
    e.ip_address
        .clone()
        .filter(|a| !a.is_empty())
        .or_else(|| e.ipam_config.as_ref().and_then(|c| c.ipv4_address.clone()))
        .unwrap_or_default()
}

/// `ubuntu.patina.local`
pub fn domain(name: &str) -> String {
    format!("{name}.{DOMAIN}")
}

/// `base`, or `base-2`, `base-3`... whichever isn't taken.
pub fn free_name(base: &str, taken: &[String]) -> String {
    let free = |n: &str| !taken.iter().any(|t| t == n);
    if free(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| {
            let tail = format!("-{i}");
            format!("{}{tail}", &base[..base.len().min(63 - tail.len())])
        })
        .find(|n| free(n))
        .unwrap()
}

/// Machines, and the containers that build their images, aren't listed
/// with containers.
pub fn hidden(c: &ContainerSummary) -> bool {
    label(c, LABEL).is_some() || label(c, BUILD_LABEL).is_some()
}

/// Your user on the Mac, as a Linux user name and uid.
pub fn user() -> (String, u32) {
    let uid = unsafe { libc::getuid() };
    let name = unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() || (*pw).pw_name.is_null() {
            None
        } else {
            std::ffi::CStr::from_ptr((*pw).pw_name).to_str().ok().map(str::to_string)
        }
    };
    let name = name.or_else(|| std::env::var("USER").ok()).unwrap_or_default().to_lowercase();
    (if valid_user(&name) { name } else { "patina".to_string() }, uid)
}

fn valid_user(s: &str) -> bool {
    let mut ch = s.chars();
    matches!(ch.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && s.len() <= 32
        && ch.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
}

/// A machine's name is also its hostname and its container's name.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The command that opens a login shell in a machine as your user.
pub fn shell_args(name: &str) -> Vec<String> {
    ["exec", "-it", "-e", "TERM", name, "su", "-", &user().0].map(String::from).to_vec()
}

/// What runs in the build container, as root: install init and sudo, add
/// the user, and keep services that would fight the engine from starting.
/// The package managers' output is left in, for [`SetupMeter`]; the
/// os-release fields come last, on a `patina-os:` line.
fn setup_script(user: &str, uid: u32) -> String {
    format!(
        r#"set -e
user='{user}'
uid='{uid}'
fail() {{ echo "$1"; exit 1; }}
net="Couldn't download packages. Check the connection, then try again."
if command -v apt-get >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive
  echo 'Acquire::Retries "3";' > /etc/apt/apt.conf.d/80-patina-retries
  apt-get update -q || fail "$net"
  apt-get install -y -q -o Dpkg::Progress-Fancy=0 --no-install-recommends systemd systemd-sysv dbus libpam-systemd sudo ca-certificates curl procps iproute2 iputils-ping nano less || fail "$net"
  apt-get clean
  rm -rf /var/lib/apt/lists/*
  # Docker's images keep services from starting when they're installed.
  rm -f /usr/sbin/policy-rc.d
  admin=sudo
elif command -v dnf >/dev/null 2>&1; then
  dnf -y install --setopt=install_weak_deps=False systemd dbus sudo util-linux shadow-utils hostname procps-ng iproute iputils nano less || fail "$net"
  dnf clean all >/dev/null 2>&1 || true
  admin=wheel
elif command -v pacman >/dev/null 2>&1; then
  # The image comes with systemd and sudo; Arch Linux ARM's mirrors are slow.
  if ! command -v sudo >/dev/null 2>&1; then
    [ -d /etc/pacman.d/gnupg ] || {{ pacman-key --init && pacman-key --populate; }} >/dev/null 2>&1
    pacman -Sy --noconfirm --needed --noprogressbar --disable-sandbox sudo || fail "$net"
  fi
  admin=wheel
elif command -v apk >/dev/null 2>&1; then
  apk add --no-cache openrc sudo curl nano less || fail "$net"
  sed -i 's/^#*rc_sys=.*/rc_sys="docker"/' /etc/rc.conf
  printf '::sysinit:/sbin/openrc sysinit\n::sysinit:/sbin/openrc boot\n::wait:/sbin/openrc default\n::shutdown:/sbin/openrc shutdown\n' > /etc/inittab
  admin=wheel
else
  echo "This distro has no package manager Patina knows."
  exit 1
fi
if ! id -u "$user" >/dev/null 2>&1; then
  if command -v useradd >/dev/null 2>&1; then
    shell=/bin/bash
    [ -x "$shell" ] || shell=/bin/sh
    useradd -m -d "/home/$user" -u "$uid" -U -s "$shell" "$user" 2>/dev/null || useradd -m -d "/home/$user" -s "$shell" "$user"
  else
    adduser -D -h "/home/$user" -u "$uid" -s /bin/sh "$user"
  fi
fi
if getent group "$admin" >/dev/null 2>&1; then
  usermod -aG "$admin" "$user" 2>/dev/null || addgroup "$user" "$admin" 2>/dev/null || true
fi
mkdir -p /etc/sudoers.d
echo "$user ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/patina
chmod 440 /etc/sudoers.d/patina
if [ -d /etc/systemd/system ]; then
  # Devices, networking, DNS and the clock belong to the engine's VM.
  for u in systemd-udevd.service systemd-udevd-control.socket systemd-udevd-kernel.socket systemd-udev-trigger.service \
      systemd-udev-settle.service systemd-networkd.service systemd-networkd.socket systemd-networkd-wait-online.service \
      systemd-resolved.service systemd-timesyncd.service systemd-modules-load.service systemd-firstboot.service \
      getty@tty1.service console-getty.service; do
    ln -sf /dev/null "/etc/systemd/system/$u"
  done
  # Each machine gets its own ID on first boot.
  rm -f /etc/machine-id
  : > /etc/machine-id
fi
# Docker provides /etc/resolv.conf; a link to resolved's would hide it.
if [ -L /etc/resolv.conf ]; then rm -f /etc/resolv.conf; : > /etc/resolv.conf; fi
. /etc/os-release
echo "patina-os: ${{ID:-}}|${{VERSION_ID:-}}|${{PRETTY_NAME:-}}"
"#
    )
}

/// Download Arch Linux's root filesystem in the VM, check it, and import it.
/// While it downloads it prints `patina-progress <bytes> <total>` each
/// second, and `patina-step <what>` before the slower steps after.
fn arch_import_script() -> String {
    format!(
        r#"set -e
site=https://images.linuxcontainers.org
arch=$(dpkg --print-architecture)
index=$(curl -fsSL "$site/meta/1.0/index-system") || {{ echo "Couldn't reach images.linuxcontainers.org. Check the connection, then try again."; exit 1; }}
path=$(printf '%s\n' "$index" | awk -F';' -v a="$arch" '$1=="archlinux" && $2=="current" && $3==a && $4=="default" {{ p=$6 }} END {{ print p }}')
[ -n "$path" ] || {{ echo "images.linuxcontainers.org has no Arch Linux for $arch."; exit 1; }}
dir=$(mktemp -d /var/tmp/patina-rootfs.XXXXXX)
trap 'rm -rf "$dir"' EXIT
cd "$dir"
url="$site${{path}}rootfs.tar.xz"
total=$(curl -fsSIL "$url" | awk 'tolower($1) == "content-length:" {{ n = $2 + 0 }} END {{ print n + 0 }}')
curl -fsSL -o rootfs.tar.xz "$url" &
pid=$!
while kill -0 "$pid" 2>/dev/null; do
  echo "patina-progress $(stat -c %s rootfs.tar.xz 2>/dev/null || echo 0) $total"
  sleep 1
done
wait "$pid" || {{ echo "Couldn't download Arch Linux. Check the connection, then try again."; exit 1; }}
echo "patina-step Checking"
curl -fsSL -o SHA256SUMS "$site${{path}}SHA256SUMS" || {{ echo "Couldn't download Arch Linux's checksum."; exit 1; }}
grep ' rootfs.tar.xz$' SHA256SUMS | sha256sum -c --status || {{ echo "The Arch Linux download didn't match its checksum. Try again."; exit 1; }}
echo "patina-step Unpacking"
docker import rootfs.tar.xz {ARCH_ROOTFS} >/dev/null
"#
    )
}

/// "Ubuntu 24.04 LTS", "Debian 13", "Fedora 45", "Alpine 3.24", "Arch Linux".
fn pretty(d: &Distro, version: &str, pretty_name: &str) -> String {
    let version = match d.key {
        "alpine" => version.split('.').take(2).collect::<Vec<_>>().join("."),
        "arch" => String::new(),
        _ => version.to_string(),
    };
    let lts = if d.key == "ubuntu" && pretty_name.contains("LTS") { " LTS" } else { "" };
    if version.is_empty() {
        d.title.to_string()
    } else {
        format!("{} {version}{lts}", d.title)
    }
}

/// The `patina-os:` line of the setup script's output.
fn os_release(out: &str) -> (String, String) {
    let line = out.lines().rev().find_map(|l| l.trim().strip_prefix("patina-os: ")).unwrap_or("");
    let mut parts = line.splitn(3, '|');
    let _id = parts.next();
    (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string())
}

fn err(e: bollard::errors::Error) -> String {
    match e {
        bollard::errors::Error::DockerResponseServerError { message, .. } => message,
        other => other.to_string(),
    }
}

/// Hears what creating a machine is doing: a sentence ("Downloading Ubuntu
/// · 18 of 29 MB") and how far along the whole thing is, when that's known.
pub type Step<'a> = &'a (dyn Fn(&str, Option<f32>) + Send + Sync);

/// The share of creating a machine each stage takes, roughly: downloading,
/// then setting up, then starting.
const DOWNLOAD: (f32, f32) = (0.0, 0.45);
const SETUP: (f32, f32) = (0.45, 0.95);

fn within(stage: (f32, f32), f: f32) -> f32 {
    stage.0 + (stage.1 - stage.0) * f.clamp(0.0, 1.0)
}

fn mb(b: f64) -> String {
    format!("{:.0}", b / 1e6)
}

/// Pull `image`, telling `progress` the bytes done and to do across its layers.
async fn pull(d: &Docker, image: &str, progress: &mut (dyn FnMut(f64, f64) + Send)) -> Result<(), String> {
    let (from, tag) = image.split_once(':').unwrap_or((image, "latest"));
    let mut s = d.create_image(Some(CreateImageOptionsBuilder::new().from_image(from).tag(tag).build()), None, None);
    let mut layers: HashMap<String, (f64, f64)> = HashMap::new();
    while let Some(item) = s.next().await {
        let info = item.map_err(|e| format!("Couldn't download {image}: {}", err(e)))?;
        if let Some(e) = info.error_detail.and_then(|e| e.message) {
            return Err(format!("Couldn't download {image}: {e}"));
        }
        let (Some(id), Some(status)) = (info.id, info.status) else { continue };
        let pd = info.progress_detail.unwrap_or_default();
        let e = layers.entry(id).or_insert((0.0, 0.0));
        match status.as_str() {
            "Downloading" => {
                if let (Some(c), Some(t)) = (pd.current, pd.total) {
                    *e = (c as f64, (t as f64).max(e.1));
                }
            }
            "Download complete" | "Pull complete" | "Already exists" | "Extracting" => e.0 = e.1,
            _ => {}
        }
        let (done, total) = layers.values().fold((0.0, 0.0), |a, l| (a.0 + l.0, a.1 + l.1));
        progress(done, total);
    }
    Ok(())
}

/// Turns package managers' output into how far setting up is: apt's
/// "N newly installed", "Get:" and "Setting up" lines, and the "[i/N]" or
/// "(i/N)" counters of dnf, apk and pacman. Only ever moves forward.
#[derive(Debug, Default)]
pub struct SetupMeter {
    apt_total: usize,
    apt_got: usize,
    apt_set: usize,
    /// dnf downloads, then runs a transaction with its own counter.
    installing: bool,
    frac: f32,
}

impl SetupMeter {
    /// The new fraction and a few words about it, when `line` moves it.
    pub fn line(&mut self, line: &str) -> Option<(f32, String)> {
        let l = line.trim();
        if let Some(n) = l.split(" newly installed").next().filter(|_| l.contains(" newly installed")).and_then(|h| h.rsplit(' ').next()).and_then(|n| n.parse().ok()) {
            self.apt_total = n;
            return None;
        }
        if l.contains("Running transaction") || l.contains("Verify package files") {
            self.installing = true;
        }
        let (f, words) = if self.apt_total > 0 && (l.starts_with("Get:") || l.starts_with("Setting up ")) {
            if l.starts_with("Get:") {
                self.apt_got += 1;
            } else {
                self.apt_set += 1;
            }
            let n = self.apt_total;
            let f = (self.apt_got.min(n) + self.apt_set.min(n)) as f32 / (2 * n) as f32;
            let words = if self.apt_set > 0 { format!("installing {} of {n}", self.apt_set.min(n)) } else { format!("downloading {} of {n}", self.apt_got.min(n)) };
            (f, words)
        } else if let Some((i, n)) = counter(l) {
            let part = i as f32 / n as f32;
            if self.installing {
                (0.5 + part / 2.0, format!("installing {i} of {n}"))
            } else {
                // apk and pacman count installs; dnf counts downloads first.
                let dnf = l.starts_with('[');
                (if dnf { part / 2.0 } else { part }, format!("{} {i} of {n}", if dnf { "downloading" } else { "installing" }))
            }
        } else {
            return None;
        };
        if f <= self.frac {
            return None;
        }
        self.frac = f.min(1.0);
        Some((self.frac, words))
    }
}

/// "[ 3/17] ..." or "(3/12) ..." at the start of a line.
fn counter(l: &str) -> Option<(usize, usize)> {
    let close = match l.chars().next()? {
        '[' => ']',
        '(' => ')',
        _ => return None,
    };
    let inner = &l[1..l.find(close)?];
    let (i, n) = inner.split_once('/')?;
    let (i, n): (usize, usize) = (i.trim().parse().ok()?, n.trim().parse().ok()?);
    (n > 0 && i <= n).then_some((i, n))
}

/// The distro's base image, built if it's missing or out of date. Returns
/// its distro label.
async fn base(d: &Docker, distro: &Distro, step: Step<'_>) -> Result<String, String> {
    let _one = BUILDING.lock().await;
    if let Ok(img) = d.inspect_image(distro.base).await {
        let labels = img.config.and_then(|c| c.labels).unwrap_or_default();
        if labels.get(BASE_LABEL).map(String::as_str) == Some(BASE_REV) {
            return Ok(labels.get(DISTRO_LABEL).cloned().unwrap_or_else(|| distro.title.to_string()));
        }
    }
    let title = distro.title;
    step(&format!("Downloading {title}"), Some(DOWNLOAD.0));
    let (source, fetched) = match distro.hub {
        Some(image) => {
            let had = d.inspect_image(image).await.is_ok();
            if !had {
                let mut last = 0.0f32;
                pull(d, image, &mut |done, total| {
                    let f = if total > 0.0 { (done / total) as f32 } else { 0.0 };
                    if total > 0.0 && f - last >= 0.01 {
                        last = f;
                        step(&format!("Downloading {title} · {} of {} MB", mb(done), mb(total)), Some(within(DOWNLOAD, f)));
                    }
                })
                .await?;
            }
            (image, !had)
        }
        None => {
            kube::in_vm_follow(d, &arch_import_script(), &mut |l| {
                let mut w = l.split_whitespace();
                match w.next() {
                    Some("patina-progress") => {
                        let done: f64 = w.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                        let total: f64 = w.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                        if total > 0.0 {
                            step(&format!("Downloading {title} · {} of {} MB", mb(done), mb(total)), Some(within(DOWNLOAD, (done / total) as f32 * 0.9)));
                        }
                    }
                    Some("patina-step") if l.ends_with("Checking") => step(&format!("Checking {title}"), Some(within(DOWNLOAD, 0.92))),
                    Some("patina-step") => step(&format!("Unpacking {title}"), Some(within(DOWNLOAD, 0.95))),
                    _ => {}
                }
            })
            .await?;
            (ARCH_ROOTFS, true)
        }
    };
    step(&format!("Setting up {title}"), Some(SETUP.0));
    let (user, uid) = user();
    let body = ContainerCreateBody {
        image: Some(source.to_string()),
        entrypoint: Some(vec!["/bin/sh".into(), "-c".into()]),
        cmd: Some(vec![setup_script(&user, uid)]),
        user: Some("0".into()),
        labels: Some(HashMap::from([(BUILD_LABEL.to_string(), distro.key.to_string())])),
        ..Default::default()
    };
    let id = d.create_container(None::<bollard::query_parameters::CreateContainerOptions>, body).await.map_err(err)?.id;
    let built = async {
        let mut meter = SetupMeter::default();
        let mut on_line = |l: &str| {
            if let Some((f, words)) = meter.line(l) {
                step(&format!("Setting up {title} · {words}"), Some(within(SETUP, f)));
            }
        };
        let out = tokio::time::timeout(SETUP_LIMIT, kube::run_follow(d, &id, &mut on_line))
            .await
            .map_err(|_| format!("Setting up {} took too long. Check the connection, then try again.", distro.title))??;
        let (version, pretty_name) = os_release(&out);
        let label = pretty(distro, &version, &pretty_name).replace(['"', '\\'], "");
        let changes = format!(
            "ENTRYPOINT [\"/sbin/init\"]\nCMD []\nSTOPSIGNAL {}\nENV container=docker\nLABEL {BASE_LABEL}=\"{BASE_REV}\" {DISTRO_LABEL}=\"{label}\"",
            distro.stop
        );
        let (repo, tag) = distro.base.rsplit_once(':').unwrap_or((distro.base, "latest"));
        let opts = CommitContainerOptionsBuilder::new().container(&id).repo(repo).tag(tag).changes(&changes).pause(false).build();
        d.commit_container(opts, bollard::models::ContainerConfig::default()).await.map_err(err)?;
        Ok::<String, String>(label)
    }
    .await;
    let _ = d.remove_container(&id, Some(RemoveContainerOptionsBuilder::new().force(true).build())).await;
    // The base shares the source's layers, so this only drops its name.
    if fetched && built.is_ok() {
        let _ = d.remove_image(source, Some(RemoveImageOptionsBuilder::new().build()), None).await;
    }
    built
}

/// Create a machine and start it. `step` hears what it's doing.
pub async fn create(d: &Docker, key: &str, name: &str, step: Step<'_>) -> Result<(), String> {
    // A machine's address comes from the VM's network (see `kube::in_vm`),
    // so say so before downloading anything.
    if super::NATIVE {
        return Err("Linux machines need Patina's VM, which Patina doesn't use on Linux yet.".into());
    }
    if !valid_name(name) {
        return Err("Use lowercase letters, digits and dashes for the name.".into());
    }
    let distro = DISTROS.iter().find(|x| x.key == key).ok_or_else(|| format!("Patina doesn't know the distro {key}."))?;
    if d.inspect_container(name, None).await.is_ok() {
        return Err(format!("A container named {name} already exists. Choose another name."));
    }
    let label = base(d, distro, step).await?;
    step("Starting", Some(SETUP.1));
    let arch = d.inspect_image(distro.base).await.ok().and_then(|i| i.architecture).unwrap_or_else(|| "arm64".into());
    let home = paths::home().display().to_string();
    let body = ContainerCreateBody {
        image: Some(distro.base.to_string()),
        hostname: Some(name.to_string()),
        labels: Some(HashMap::from([
            (LABEL.to_string(), name.to_string()),
            (DISTRO_LABEL.to_string(), label),
            (ARCH_LABEL.to_string(), arch),
        ])),
        stop_timeout: Some(20),
        host_config: Some(HostConfig {
            // Init needs to manage cgroups and mounts, as on a real system.
            privileged: Some(true),
            cgroupns_mode: Some(HostConfigCgroupnsModeEnum::PRIVATE),
            tmpfs: Some(HashMap::from([
                ("/run".to_string(), "exec,mode=755".to_string()),
                ("/run/lock".to_string(), "mode=1777".to_string()),
            ])),
            binds: Some(vec![format!("{home}:{home}")]),
            // Running machines come back with the engine.
            restart_policy: Some(RestartPolicy { name: Some(RestartPolicyNameEnum::UNLESS_STOPPED), maximum_retry_count: None }),
            ..Default::default()
        }),
        ..Default::default()
    };
    d.create_container(Some(CreateContainerOptionsBuilder::new().name(name).build()), body).await.map_err(err)?;
    if let Err(e) = start(d, name).await {
        let _ = d.remove_container(name, Some(RemoveContainerOptionsBuilder::new().force(true).build())).await;
        return Err(e);
    }
    Ok(())
}

/// Start a machine and make sure its init keeps running. A machine without
/// an address (from before addresses, or after the network was removed)
/// gets one first.
pub async fn start(d: &Docker, name: &str) -> Result<(), String> {
    if let Err(e) = connect_lan(d, name).await {
        // The machine still works without one.
        if std::env::var_os("PATINA_DEBUG").is_some() {
            eprintln!("[patina] address for {name}: {e}");
        }
    }
    d.start_container(name, None).await.map_err(err)?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let running = d.inspect_container(name, None).await.ok().and_then(|c| c.state).and_then(|s| s.running).unwrap_or(false);
    if running {
        Ok(())
    } else {
        Err(format!("{name} stopped while it was starting. Try starting it again."))
    }
}

/// The VM's network: its address, prefix length and gateway.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Net {
    base: Ipv4Addr,
    prefix: u8,
    gateway: Ipv4Addr,
}

impl Net {
    fn contains(&self, ip: Ipv4Addr) -> bool {
        mask(u32::from(ip), self.prefix) == u32::from(self.base)
    }
    /// Machines take addresses from the top of the network down, away from
    /// where macOS starts lending them to VMs (.2 up). At most the top 64.
    fn candidates(&self) -> impl Iterator<Item = Ipv4Addr> {
        let size = 1u64 << (32 - self.prefix as u32);
        let base = u32::from(self.base) as u64;
        let top = base + size - 2; // below the broadcast address
        let low = (base + 2).max(top.saturating_sub(62));
        (low..=top).rev().map(|v| Ipv4Addr::from(v as u32))
    }
}

fn mask(v: u32, prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        v & (u32::MAX << (32 - prefix as u32))
    }
}

/// "enp0s1 192.168.64.5/24 192.168.64.1" from the VM.
fn parse_vm_net(out: &str) -> Option<(String, Net, Ipv4Addr)> {
    let mut w = out.split_whitespace();
    let dev = w.next()?.to_string();
    let (addr, prefix) = w.next()?.split_once('/')?;
    let gateway: Ipv4Addr = w.next()?.parse().ok()?;
    let vm: Ipv4Addr = addr.parse().ok()?;
    let prefix: u8 = prefix.parse().ok().filter(|p| (8..=30).contains(p))?;
    Some((dev, Net { base: Ipv4Addr::from(mask(u32::from(vm), prefix)), prefix, gateway }, vm))
}

/// Addresses macOS has lent to VMs.
fn leased() -> Vec<Ipv4Addr> {
    std::fs::read_to_string(LEASES)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix("ip_address=")?.parse().ok())
        .collect()
}

const VM_NET: &str = r#"dev=$(ip -4 route show default | awk '{ for (i = 1; i < NF; i++) if ($i == "dev") { print $(i + 1); exit } }')
gw=$(ip -4 route show default | awk '{ print $3; exit }')
addr=$(ip -4 -o addr show dev "$dev" scope global | awk '{ print $4; exit }')
echo "$dev $addr $gw""#;

/// The machines' network, made the first time.
async fn lan(d: &Docker) -> Result<Net, String> {
    let read = |n: bollard::models::NetworkInspect| -> Option<Net> {
        let c = n.ipam?.config?.into_iter().next()?;
        let (base, prefix) = c.subnet?.split_once('/').map(|(a, p)| (a.parse().ok(), p.parse().ok()))?;
        Some(Net { base: base?, prefix: prefix?, gateway: c.gateway?.parse().ok()? })
    };
    if let Ok(n) = d.inspect_network(LAN, None).await {
        return read(n).ok_or_else(|| format!("The {LAN} network has no subnet."));
    }
    let out = kube::in_vm(d, VM_NET).await?;
    let (dev, net, _) = parse_vm_net(&out).ok_or_else(|| format!("Couldn't read the VM's network: {}", out.trim()))?;
    let req = NetworkCreateRequest {
        name: LAN.to_string(),
        driver: Some("ipvlan".into()),
        options: Some(HashMap::from([("parent".to_string(), dev), ("ipvlan_mode".to_string(), "l2".to_string())])),
        ipam: Some(Ipam {
            config: Some(vec![IpamConfig {
                subnet: Some(format!("{}/{}", net.base, net.prefix)),
                gateway: Some(net.gateway.to_string()),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        labels: Some(HashMap::from([(LABEL.to_string(), "network".to_string())])),
        ..Default::default()
    };
    match d.create_network(req).await {
        Ok(_) => Ok(net),
        // Made at the same time by another create.
        Err(e) => match d.inspect_network(LAN, None).await {
            Ok(n) => read(n).ok_or_else(|| err(e)),
            Err(_) => Err(err(e)),
        },
    }
}

/// Give a machine its address, if it doesn't have one. Returns it.
pub async fn connect_lan(d: &Docker, name: &str) -> Result<String, String> {
    let _one = ADDRESSING.lock().await;
    let c = d.inspect_container(name, None).await.map_err(err)?;
    if let Some(e) = c.network_settings.and_then(|n| n.networks).and_then(|mut n| n.remove(LAN)) {
        return Ok(address(&e));
    }
    let net = lan(d).await?;
    let mut taken: Vec<Ipv4Addr> = leased();
    taken.push(net.gateway);
    let opts = ListContainersOptionsBuilder::new().all(true).build();
    for c in d.list_containers(Some(opts)).await.map_err(err)? {
        let nets = c.network_settings.and_then(|n| n.networks).unwrap_or_default();
        taken.extend(nets.values().filter_map(|e| address(e).parse::<Ipv4Addr>().ok()));
    }
    let ip = net
        .candidates()
        .find(|ip| net.contains(*ip) && !taken.contains(ip))
        .ok_or_else(|| "Every address for machines is in use.".to_string())?;
    let req = NetworkConnectRequest {
        container: name.to_string(),
        endpoint_config: Some(EndpointSettings {
            ipam_config: Some(EndpointIpamConfig { ipv4_address: Some(ip.to_string()), ..Default::default() }),
            ..Default::default()
        }),
    };
    d.connect_network(LAN, req).await.map_err(err)?;
    Ok(ip.to_string())
}

/// What the details dialog shows beyond the list: disk used by the
/// machine's own files, memory in use, and when it was created.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub disk_mb: f64,
    pub mem_mb: Option<f64>,
    /// Unix time.
    pub created: Option<i64>,
}

pub async fn details(d: &Docker, name: &str) -> Result<Details, String> {
    let c = d.inspect_container(name, Some(InspectContainerOptionsBuilder::new().size(true).build())).await.map_err(err)?;
    let running = c.state.as_ref().and_then(|s| s.running).unwrap_or(false);
    let mem_mb = if running {
        let opts = bollard::query_parameters::StatsOptionsBuilder::new().stream(false).one_shot(true).build();
        let mut s = d.stats(name, Some(opts));
        s.next().await.and_then(|r| r.ok()).and_then(|st| st.memory_stats).and_then(|m| {
            let usage = m.usage? as f64;
            let cache = m.stats.as_ref().and_then(|s| s.get("inactive_file").copied()).unwrap_or(0) as f64;
            Some((usage - cache).max(0.0) / 1e6)
        })
    } else {
        None
    };
    let created = c.created.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.timestamp());
    Ok(Details { disk_mb: c.size_rw.unwrap_or(0) as f64 / 1e6, mem_mb, created })
}

pub async fn stop(d: &Docker, name: &str) -> Result<(), String> {
    d.stop_container(name, None).await.map_err(err)
}

pub async fn delete(d: &Docker, name: &str) -> Result<(), String> {
    d.remove_container(name, Some(RemoveContainerOptionsBuilder::new().force(true).v(true).build())).await.map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_every_distro_the_ui_offers() {
        let keys: Vec<_> = DISTROS.iter().map(|d| d.key).collect();
        let ui: Vec<_> = crate::sim::DISTROS.iter().map(|d| d.0).collect();
        assert_eq!(keys, ui);
    }

    #[test]
    fn names_are_hostnames() {
        assert!(valid_name("ubuntu"));
        assert!(valid_name("dev-2"));
        assert!(!valid_name(""));
        assert!(!valid_name("-dev"));
        assert!(!valid_name("dev-"));
        assert!(!valid_name("Dev"));
        assert!(!valid_name("my_box"));
        assert!(!valid_name(&"a".repeat(64)));
    }

    #[test]
    fn user_names_are_linux_names() {
        assert!(valid_user("sadat"));
        assert!(valid_user("j.doe"));
        assert!(!valid_user("1abc"));
        assert!(!valid_user("a b"));
        assert!(!valid_user(""));
    }

    #[test]
    fn names_distros_like_the_design() {
        let by = |k: &str| DISTROS.iter().find(|d| d.key == k).unwrap();
        assert_eq!(pretty(by("ubuntu"), "24.04", "Ubuntu 24.04.5 LTS"), "Ubuntu 24.04 LTS");
        assert_eq!(pretty(by("debian"), "13", "Debian GNU/Linux 13 (trixie)"), "Debian 13");
        assert_eq!(pretty(by("fedora"), "45", "Fedora Linux 45 (Container Image)"), "Fedora 45");
        assert_eq!(pretty(by("alpine"), "3.24.4", "Alpine Linux v3.24"), "Alpine 3.24");
        assert_eq!(pretty(by("arch"), "", "Arch Linux ARM"), "Arch Linux");
        assert_eq!(pretty(by("fedora"), "", ""), "Fedora");
    }

    #[test]
    fn reads_the_os_release_line() {
        let out = "Setting up...\npatina-os: ubuntu|24.04|Ubuntu 24.04.5 LTS\n";
        assert_eq!(os_release(out), ("24.04".to_string(), "Ubuntu 24.04.5 LTS".to_string()));
        assert_eq!(os_release("nothing"), (String::new(), String::new()));
    }

    #[test]
    fn scripts_are_filled_in() {
        let s = setup_script("sadat", 501);
        assert!(s.contains("user='sadat'") && s.contains("uid='501'"));
        assert!(s.contains("${ID:-}") && !s.contains("{{"));
        let a = arch_import_script();
        assert!(a.contains(ARCH_ROOTFS) && a.contains("sha256sum -c") && !a.contains("{{"));
    }

    #[test]
    fn meters_apt() {
        let mut m = SetupMeter::default();
        assert_eq!(m.line("Get:1 http://ports.ubuntu.com noble InRelease [256 kB]"), None);
        assert_eq!(m.line("0 upgraded, 4 newly installed, 0 to remove and 0 not upgraded."), None);
        assert_eq!(m.line("Get:1 http://ports.ubuntu.com noble/main arm64 libfoo [1 kB]"), Some((0.125, "downloading 1 of 4".into())));
        for _ in 0..3 {
            m.line("Get:2 http://x y");
        }
        assert_eq!(m.line("Setting up libfoo (1.0) ..."), Some((0.625, "installing 1 of 4".into())));
        for _ in 0..3 {
            m.line("Setting up libbar (1.0) ...");
        }
        assert_eq!(m.frac, 1.0);
    }

    #[test]
    fn meters_dnf_and_apk() {
        let mut m = SetupMeter::default();
        assert_eq!(m.line("[ 3/10] systemd-0:258.2-1.fc45.aarch64 100% | 6.0 MiB/s"), Some((0.15, "downloading 3 of 10".into())));
        m.line("Running transaction");
        assert_eq!(m.line("[ 2/20] Installing dbus-1:1.16 100%"), Some((0.55, "installing 2 of 20".into())));
        // It never goes back.
        assert_eq!(m.line("[ 1/20] Verify package files 100%"), None);
        let mut a = SetupMeter::default();
        assert_eq!(a.line("(3/12) Installing sudo (1.9.17-r0)"), Some((0.25, "installing 3 of 12".into())));
        assert_eq!(a.line("fetch https://dl-cdn.alpinelinux.org/alpine/v3.24/main"), None);
    }

    #[test]
    fn picks_a_free_default_name() {
        let taken = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(free_name("ubuntu", &taken(&[])), "ubuntu");
        assert_eq!(free_name("ubuntu", &taken(&["ubuntu"])), "ubuntu-2");
        assert_eq!(free_name("ubuntu", &taken(&["ubuntu", "ubuntu-2", "ubuntu-3"])), "ubuntu-4");
        let long = "a".repeat(63);
        let n = free_name(&long, &taken(&[&long]));
        assert!(n.len() <= 63 && n.ends_with("-2") && valid_name(&n));
    }

    #[test]
    fn reads_the_vm_network() {
        let (dev, net, vm) = parse_vm_net("enp0s1 192.168.64.5/24 192.168.64.1\n").unwrap();
        assert_eq!(dev, "enp0s1");
        assert_eq!(vm, Ipv4Addr::new(192, 168, 64, 5));
        assert_eq!(net, Net { base: Ipv4Addr::new(192, 168, 64, 0), prefix: 24, gateway: Ipv4Addr::new(192, 168, 64, 1) });
        assert!(parse_vm_net("").is_none());
        assert!(parse_vm_net("enp0s1 192.168.64.5/32 192.168.64.1").is_none());
    }

    #[test]
    fn hands_out_addresses_from_the_top() {
        let net = Net { base: Ipv4Addr::new(192, 168, 64, 0), prefix: 24, gateway: Ipv4Addr::new(192, 168, 64, 1) };
        let all: Vec<_> = net.candidates().collect();
        assert_eq!(all.first(), Some(&Ipv4Addr::new(192, 168, 64, 254)));
        assert_eq!(all.last(), Some(&Ipv4Addr::new(192, 168, 64, 192)));
        assert_eq!(all.len(), 63);
        let small = Net { base: Ipv4Addr::new(10, 0, 0, 0), prefix: 28, gateway: Ipv4Addr::new(10, 0, 0, 1) };
        let all: Vec<_> = small.candidates().collect();
        assert_eq!(all, (2..=14).rev().map(|i| Ipv4Addr::new(10, 0, 0, i)).collect::<Vec<_>>());
    }

    #[test]
    fn the_shell_logs_in_as_you() {
        let args = shell_args("dev");
        assert_eq!(&args[..5], ["exec", "-it", "-e", "TERM", "dev"]);
        assert_eq!(args[5..7], ["su", "-"]);
    }
}
