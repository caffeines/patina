//! Bring images, volumes and containers over from another Docker engine on
//! this Mac (Docker Desktop, Colima or OrbStack), through the Docker API.
//!
//! The other engine is only read from:
//! - images stream out of `docker save` straight into `docker load`;
//! - each volume is read through a container that is created but never
//!   started, then written into a same-named volume the same way (the
//!   helper containers are removed again);
//! - containers are recreated from their settings, stopped, so they don't
//!   fight the other engine over published ports. Files changed inside a
//!   container (outside its volumes) aren't copied.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bollard::errors::Error;
use bollard::models::{ContainerCreateBody, ContainerSummary, EndpointSettings, HostConfig, ImageSummary, NetworkCreateRequest, NetworkingConfig, Volume, VolumeCreateRequest};
use bollard::query_parameters::{
    CreateContainerOptions, DownloadFromContainerOptions, ImportImageOptions, ListContainersOptionsBuilder, ListImagesOptions, ListNetworksOptions,
    ListVolumesOptions, RemoveContainerOptions, UploadToContainerOptions,
};
use bollard::{body_try_stream, Docker, API_DEFAULT_VERSION};
use futures_util::{StreamExt, TryStreamExt};

use super::paths;
use super::setup::Report;

/// Another engine found on this Mac.
#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub sock: PathBuf,
    /// Its Docker API answered. When false the counts are zero.
    pub running: bool,
    pub images: usize,
    pub volumes: usize,
    pub containers: usize,
}

#[derive(Debug, Default)]
pub struct Summary {
    pub images: usize,
    pub volumes: usize,
    pub containers: usize,
    /// "what: why" for everything that couldn't be copied.
    pub failed: Vec<String>,
}

/// Installed engines, most likely first: (name, socket).
fn candidates() -> Vec<(String, PathBuf)> {
    let home = paths::home();
    let mut out = Vec::new();
    if std::path::Path::new("/Applications/Docker.app").exists() || home.join(".docker/run/docker.sock").exists() {
        out.push(("Docker Desktop".to_string(), home.join(".docker/run/docker.sock")));
    }
    let colima = std::env::var_os("COLIMA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".colima"));
    if let Ok(dir) = std::fs::read_dir(&colima) {
        let mut profiles: Vec<String> = dir.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('_')).collect();
        profiles.sort_by_key(|p| p != "default");
        for p in profiles {
            let name = if p == "default" { "Colima".to_string() } else { format!("Colima ({p})") };
            out.push((name, colima.join(&p).join("docker.sock")));
        }
    }
    if std::path::Path::new("/Applications/OrbStack.app").exists() || home.join(".orbstack").exists() {
        out.push(("OrbStack".to_string(), home.join(".orbstack/run/docker.sock")));
    }
    out
}

async fn connect(sock: &std::path::Path, timeout: u64) -> Result<Docker, Error> {
    Docker::connect_with_unix(&sock.to_string_lossy(), timeout, API_DEFAULT_VERSION)?.negotiate_version().await
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime")
}

/// The first running engine with something to bring over, else the first
/// installed one (not running), else `None`.
pub fn detect() -> Option<Source> {
    runtime().block_on(async {
        let mut idle = None;
        for (name, sock) in candidates() {
            let found = match tokio::time::timeout(Duration::from_secs(5), async { plan(&connect(&sock, 5).await?).await }).await {
                Ok(Ok(p)) => Some(p),
                _ => None,
            };
            let mut s = Source { name, sock, running: found.is_some(), images: 0, volumes: 0, containers: 0 };
            match found {
                Some(p) if !p.is_empty() => {
                    (s.images, s.volumes, s.containers) = (p.images.len(), p.volumes.len(), p.containers.len());
                    return Some(s);
                }
                Some(_) => {} // running but empty: nothing to offer
                None if idle.is_none() => idle = Some(s),
                None => {}
            }
        }
        idle
    })
}

/// What an import would copy.
struct Plan {
    images: Vec<ImageSummary>,
    volumes: Vec<Volume>,
    containers: Vec<ContainerSummary>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.images.is_empty() && self.volumes.is_empty() && self.containers.is_empty()
    }
}

fn tags(img: &ImageSummary) -> Vec<String> {
    img.repo_tags.iter().filter(|t| t.as_str() != "<none>:<none>").cloned().collect()
}

/// Volumes Docker names itself (64 hex characters).
fn anonymous(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

fn kubernetes(c: &ContainerSummary) -> bool {
    c.labels.as_ref().is_some_and(|l| l.contains_key("io.kubernetes.pod.name"))
}

async fn plan(d: &Docker) -> Result<Plan, Error> {
    let mut images: Vec<ImageSummary> = d.list_images(None::<ListImagesOptions>).await?.into_iter().filter(|i| !tags(i).is_empty()).collect();
    images.sort_by_key(|i| i.size);
    let containers: Vec<ContainerSummary> =
        d.list_containers(Some(ListContainersOptionsBuilder::new().all(true).build())).await?.into_iter().filter(|c| !kubernetes(c)).collect();
    let used: HashSet<String> = containers.iter().flat_map(|c| c.mounts.iter().flatten()).filter_map(|m| m.name.clone()).collect();
    // Named volumes, and anonymous ones a container still uses.
    let volumes = d
        .list_volumes(None::<ListVolumesOptions>)
        .await?
        .volumes
        .unwrap_or_default()
        .into_iter()
        .filter(|v| v.driver == "local" && (!anonymous(&v.name) || used.contains(&v.name)))
        .collect();
    Ok(Plan { images, volumes, containers })
}

/// Copy everything from `src` into Patina's engine. Partial failures are
/// listed in the summary; `Err` means nothing could be done.
pub fn run(src: &Source, report: Report) -> Result<Summary, String> {
    runtime().block_on(async {
        let from = connect(&src.sock, 3600).await.map_err(|e| format!("Couldn't reach {}: {e}", src.name))?;
        let to = connect(&paths::docker_sock(), 3600).await.map_err(|e| format!("Couldn't reach Patina's engine: {e}"))?;
        let plan = plan(&from).await.map_err(|e| format!("Couldn't list what's in {}: {e}", src.name))?;
        let total = (plan.images.len() + plan.volumes.len() + plan.containers.len()).max(1) as f64;
        let mut done = 0.0;
        let mut sum = Summary::default();

        // Images first: volume helpers and containers need them.
        let mut have: HashSet<String> = to.list_images(None::<ListImagesOptions>).await.map_err(|e| e.to_string())?.iter().flat_map(tags).collect();
        for img in &plan.images {
            let names = tags(img);
            if names.iter().all(|t| have.contains(t)) {
                sum.images += 1;
            } else {
                let at = done / total;
                let step = 1.0 / total;
                let size = img.size.max(1) as f64;
                let tick = |n: u64| report(at + step * (n as f64 / size).min(0.99), &format!("Copying {}  {:.0} MB", names[0], n as f64 / 1e6));
                match copy_image(&from, &to, &names, tick).await {
                    Ok(()) => {
                        sum.images += 1;
                        have.extend(names);
                    }
                    Err(e) => sum.failed.push(format!("{}: {e}", names[0])),
                }
            }
            done += 1.0;
            report(done / total, "Copying images");
        }

        // A small image both engines have, to create the volume helpers from.
        let helper = plan.images.iter().filter_map(|i| tags(i).into_iter().next()).find(|t| have.contains(t));
        let existing: HashSet<String> = to.list_volumes(None::<ListVolumesOptions>).await.ok().and_then(|r| r.volumes).unwrap_or_default().into_iter().map(|v| v.name).collect();
        for v in &plan.volumes {
            report(done / total, &format!("Copying volume {}", short(&v.name)));
            let r = match &helper {
                _ if existing.contains(&v.name) => Ok(()),
                Some(h) => copy_volume(&from, &to, v, h).await.map_err(|e| e.to_string()),
                None => Err("no image to copy it with".to_string()),
            };
            match r {
                Ok(()) => sum.volumes += 1,
                Err(e) => sum.failed.push(format!("volume {}: {e}", short(&v.name))),
            }
            done += 1.0;
        }

        // Networks the containers use, then the containers, stopped.
        copy_networks(&from, &to, &mut sum).await;
        let names: HashMap<String, String> = plan.containers.iter().filter_map(|c| Some((c.id.clone()?, name(c)?))).collect();
        let taken: HashSet<String> =
            to.list_containers(Some(ListContainersOptionsBuilder::new().all(true).build())).await.unwrap_or_default().iter().filter_map(name).collect();
        for c in &plan.containers {
            let Some(n) = name(c) else { continue };
            report(done / total, &format!("Creating {n}"));
            if taken.contains(&n) {
                sum.containers += 1;
            } else {
                match copy_container(&from, &to, c, &names).await {
                    Ok(()) => sum.containers += 1,
                    Err(e) => sum.failed.push(format!("{n}: {e}")),
                }
            }
            done += 1.0;
        }
        report(1.0, "done");
        Ok(sum)
    })
}

fn short(name: &str) -> &str {
    if anonymous(name) {
        &name[..12]
    } else {
        name
    }
}

fn name(c: &ContainerSummary) -> Option<String> {
    Some(c.names.as_ref()?.first()?.trim_start_matches('/').to_string())
}

/// `docker save` on one side piped into `docker load` on the other, calling
/// `tick` with the bytes moved so far.
async fn copy_image(from: &Docker, to: &Docker, names: &[String], tick: impl Fn(u64)) -> Result<(), Error> {
    let moved = Arc::new(AtomicU64::new(0));
    let m = moved.clone();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let tar = from.export_images(&refs).inspect_ok(move |b| {
        m.fetch_add(b.len() as u64, Ordering::Relaxed);
    });
    let mut load = to.import_image_stream(ImportImageOptions { quiet: true, ..Default::default() }, tar, None);
    let mut every = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            r = load.next() => match r {
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e),
                None => return Ok(()),
            },
            _ = every.tick() => tick(moved.load(Ordering::Relaxed)),
        }
    }
}

/// A container that's never started, with `vol` mounted at `/v`.
async fn helper(d: &Docker, image: &str, vol: &str) -> Result<String, Error> {
    let body = ContainerCreateBody {
        image: Some(image.to_string()),
        cmd: Some(vec!["true".into()]),
        entrypoint: Some(vec![]),
        network_disabled: Some(true),
        labels: Some(HashMap::from([("dev.patina.import".to_string(), "helper".to_string())])),
        host_config: Some(HostConfig { binds: Some(vec![format!("{vol}:/v")]), ..Default::default() }),
        ..Default::default()
    };
    Ok(d.create_container(None::<CreateContainerOptions>, body).await?.id)
}

/// Remove a helper. `v` drops only anonymous volumes the image declared,
/// never the named volume mounted at `/v`.
async fn drop_helper(d: &Docker, id: &str) {
    let _ = d.remove_container(id, Some(RemoveContainerOptions { force: true, v: true, link: false })).await;
}

async fn copy_volume(from: &Docker, to: &Docker, v: &Volume, image: &str) -> Result<(), Error> {
    let req = VolumeCreateRequest {
        name: Some(v.name.clone()),
        driver: Some("local".into()),
        driver_opts: (!v.options.is_empty()).then(|| v.options.clone()),
        labels: Some(v.labels.clone()),
        ..Default::default()
    };
    to.create_volume(req).await?;
    let a = helper(from, image, &v.name).await?;
    let b = match helper(to, image, &v.name).await {
        Ok(b) => b,
        Err(e) => {
            drop_helper(from, &a).await;
            return Err(e);
        }
    };
    // The archive of /v holds `v/...`; unpacked at / it lands in the volume,
    // owners and modes included.
    let tar = from.download_from_container(&a, Some(DownloadFromContainerOptions { path: "/v".into() })).map_err(std::io::Error::other);
    let r = to.upload_to_container(&b, Some(UploadToContainerOptions { path: "/".into(), ..Default::default() }), body_try_stream(tar)).await;
    drop_helper(from, &a).await;
    drop_helper(to, &b).await;
    r
}

const BUILTIN_NETWORKS: [&str; 3] = ["bridge", "host", "none"];

/// User-defined bridge networks, with their labels (so Compose still owns
/// them) and subnets when those are free.
async fn copy_networks(from: &Docker, to: &Docker, sum: &mut Summary) {
    let Ok(theirs) = from.list_networks(None::<ListNetworksOptions>).await else { return };
    let ours: HashSet<String> = to.list_networks(None::<ListNetworksOptions>).await.unwrap_or_default().into_iter().filter_map(|n| n.name).collect();
    for n in theirs {
        let Some(name) = n.name.clone() else { continue };
        if BUILTIN_NETWORKS.contains(&name.as_str()) || ours.contains(&name) || n.driver.as_deref() != Some("bridge") {
            continue;
        }
        let mut req = NetworkCreateRequest {
            name: name.clone(),
            driver: n.driver,
            internal: n.internal,
            attachable: n.attachable,
            enable_ipv6: n.enable_ipv6,
            ipam: n.ipam,
            options: n.options,
            labels: n.labels,
            ..Default::default()
        };
        if to.create_network(req.clone()).await.is_err() {
            // Most likely the subnet is taken here; let Docker pick one.
            req.ipam = None;
            if let Err(e) = to.create_network(req).await {
                sum.failed.push(format!("network {name}: {e}"));
            }
        }
    }
}

/// Recreate a container from its settings, without starting it.
async fn copy_container(from: &Docker, to: &Docker, c: &ContainerSummary, names: &HashMap<String, String>) -> Result<(), Error> {
    let id = c.id.clone().unwrap_or_default();
    let info = from.inspect_container(&id, None).await?;
    let cfg = info.config.unwrap_or_default();
    let mut hc = info.host_config.unwrap_or_default();
    hc.container_id_file = None;
    // `container:<id>` network modes refer to the other engine's IDs.
    if let Some(other) = hc.network_mode.as_deref().and_then(|m| m.strip_prefix("container:")) {
        let other = names.iter().find(|(k, _)| k.starts_with(other)).map(|(_, n)| n.clone()).unwrap_or_else(|| other.to_string());
        hc.network_mode = Some(format!("container:{other}"));
    }
    // Keep anonymous volumes an image declared on the same data.
    let mut binds = hc.binds.clone().unwrap_or_default();
    let taken: HashSet<String> =
        binds.iter().filter_map(|b| b.split(':').nth(1).map(str::to_string)).chain(hc.mounts.iter().flatten().filter_map(|m| m.target.clone())).collect();
    for m in info.mounts.iter().flatten() {
        if let (Some("volume"), Some(vol), Some(dst)) = (m.typ.as_deref(), m.name.as_deref(), m.destination.as_deref()) {
            if anonymous(vol) && !taken.contains(dst) {
                binds.push(format!("{vol}:{dst}{}", if m.rw == Some(false) { ":ro" } else { "" }));
            }
        }
    }
    hc.binds = (!binds.is_empty()).then_some(binds);

    let mode = hc.network_mode.clone().unwrap_or_default();
    let own_net = !(mode == "host" || mode == "none" || mode.starts_with("container:"));
    let endpoints: HashMap<String, EndpointSettings> = info
        .network_settings
        .and_then(|n| n.networks)
        .unwrap_or_default()
        .into_iter()
        .map(|(net, e)| (net, EndpointSettings { aliases: e.aliases, links: e.links, ipam_config: e.ipam_config, driver_opts: e.driver_opts, ..Default::default() }))
        .collect();
    // Docker sets the hostname to the short ID unless one was given.
    let hostname = cfg.hostname.filter(|h| !id.starts_with(h.as_str()));

    let body = ContainerCreateBody {
        hostname,
        domainname: cfg.domainname,
        user: cfg.user,
        attach_stdin: cfg.attach_stdin,
        attach_stdout: cfg.attach_stdout,
        attach_stderr: cfg.attach_stderr,
        exposed_ports: cfg.exposed_ports,
        tty: cfg.tty,
        open_stdin: cfg.open_stdin,
        stdin_once: cfg.stdin_once,
        env: cfg.env,
        cmd: cfg.cmd,
        healthcheck: cfg.healthcheck,
        args_escaped: cfg.args_escaped,
        image: cfg.image,
        volumes: cfg.volumes,
        working_dir: cfg.working_dir,
        entrypoint: cfg.entrypoint,
        network_disabled: cfg.network_disabled,
        on_build: cfg.on_build,
        labels: cfg.labels,
        stop_signal: cfg.stop_signal,
        stop_timeout: cfg.stop_timeout,
        shell: cfg.shell,
        host_config: Some(hc),
        networking_config: (own_net && !endpoints.is_empty()).then_some(NetworkingConfig { endpoints_config: Some(endpoints) }),
    };
    let name = names.get(&id).cloned();
    to.create_container(Some(CreateContainerOptions { name, ..Default::default() }), body).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_anonymous_volumes() {
        assert!(anonymous(&"a1".repeat(32)));
        assert!(!anonymous("pgdata"));
        assert!(!anonymous(&"z".repeat(64)));
    }

    #[test]
    fn untagged_images_have_no_tags() {
        let img = ImageSummary { repo_tags: vec!["<none>:<none>".into()], ..Default::default() };
        assert!(tags(&img).is_empty());
        let img = ImageSummary { repo_tags: vec!["postgres:17".into(), "db:latest".into()], ..Default::default() };
        assert_eq!(tags(&img), ["postgres:17", "db:latest"]);
    }
}
