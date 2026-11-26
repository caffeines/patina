//! What takes up the engine's disk, and cleaning up what nothing uses.
//!
//! [`usage`] reads Docker's disk usage (`/system/df`) and sorts it into what
//! the dashboard and the Clean up dialog show: images (and which are
//! dangling, unused, or machine base images no machine uses), containers,
//! machines, volumes, build cache and networks. It reads the response itself
//! because the engine's Docker (26) answers in the older shape that bollard's
//! newer model doesn't parse; both shapes are understood.
//!
//! [`clean`] removes what's chosen. It never touches machines (their files
//! are someone's work) or Kubernetes' pod containers (the cluster manages
//! them), and only removes images, volumes and networks no container uses.

use std::collections::HashSet;

use bollard::query_parameters::{
    ListContainersOptionsBuilder, ListImagesOptionsBuilder, PruneBuildOptionsBuilder, PruneImagesOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveImageOptionsBuilder,
};
use bollard::Docker;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{kube, machines, paths};

/// A count and the bytes it takes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Part {
    pub n: usize,
    pub bytes: f64,
}

impl Part {
    fn add(&mut self, bytes: f64) {
        self.n += 1;
        self.bytes += bytes.max(0.0);
    }
}

/// One image or volume for the "biggest" lists.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    pub bytes: f64,
    pub used: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// Every image. Bytes count shared layers once.
    pub images: Part,
    /// Tagged images no container uses (machine base images aside).
    pub images_unused: Part,
    /// Untagged images no container uses.
    pub dangling: Part,
    /// Machine base images (`patina.local/...`), and those no machine uses.
    pub distros: Part,
    pub distros_unused: Part,
    /// Your containers (not machines, pods or Patina's helpers): their own files.
    pub containers: Part,
    pub running: usize,
    pub stopped: Part,
    /// Kubernetes' pod containers.
    pub pods: Part,
    /// Machines: the files in each machine.
    pub machines: Part,
    pub volumes: Part,
    pub volumes_unused: Part,
    pub cache: Part,
    pub cache_unused: Part,
    /// Networks you or Compose made, and those no container uses.
    pub networks: usize,
    pub networks_unused: usize,
    pub biggest_images: Vec<Item>,
    pub biggest_volumes: Vec<Item>,
    /// Every volume's size, for the Volumes list.
    pub volume_bytes: Vec<(String, f64)>,
}

impl Usage {
    /// Everything on the engine's disk that Docker accounts for.
    pub fn total(&self) -> f64 {
        self.images.bytes + self.containers.bytes + self.pods.bytes + self.machines.bytes + self.volumes.bytes + self.cache.bytes
    }

    /// What cleaning up `sel` would free, at most.
    pub fn freeable(&self, sel: &Sel) -> f64 {
        let mut b = 0.0;
        if sel.stopped {
            b += self.stopped.bytes;
        }
        if sel.dangling {
            b += self.dangling.bytes;
        }
        if sel.images {
            b += self.images_unused.bytes;
        }
        if sel.distros {
            b += self.distros_unused.bytes;
        }
        if sel.cache {
            b += self.cache_unused.bytes;
        }
        if sel.volumes {
            b += self.volumes_unused.bytes;
        }
        b
    }

    /// Everything that cleaning up could free, volumes aside.
    pub fn reclaimable(&self) -> f64 {
        self.freeable(&Sel { volumes: false, ..Sel::all() })
    }
}

/// What to clean up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sel {
    pub stopped: bool,
    pub dangling: bool,
    pub images: bool,
    pub distros: bool,
    pub cache: bool,
    pub networks: bool,
    pub volumes: bool,
}

impl Sel {
    pub fn all() -> Sel {
        Sel { stopped: true, dangling: true, images: true, distros: true, cache: true, networks: true, volumes: true }
    }
    pub fn any(&self) -> bool {
        self.stopped || self.dangling || self.images || self.distros || self.cache || self.networks || self.volumes
    }
}

impl Default for Sel {
    /// What nothing needs: not stopped containers (Compose brings its own
    /// back with their state gone) or volumes (their data would be lost).
    fn default() -> Sel {
        Sel { stopped: false, volumes: false, ..Sel::all() }
    }
}

fn labels(v: &Value) -> impl Iterator<Item = &str> {
    v["Labels"].as_object().into_iter().flat_map(|m| m.keys().map(String::as_str))
}

fn has_label(v: &Value, key: &str) -> bool {
    labels(v).any(|k| k == key)
}

fn tags(v: &Value) -> Vec<String> {
    v["RepoTags"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|t| *t != "<none>:<none>").map(str::to_string).collect()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn is_distro(tags: &[String]) -> bool {
    tags.iter().any(|t| t.starts_with("patina.local/"))
}

/// A container Patina keeps out of cleaning and out of "your containers":
/// machines, pods, and Patina's own helpers and machine builds.
fn kept(c: &Value) -> bool {
    has_label(c, machines::LABEL) || has_label(c, "io.kubernetes.pod.name") || has_label(c, kube::HELPER_LABEL) || has_label(c, machines::BUILD_LABEL)
}

/// Sort `/system/df` into a [`Usage`]. `used_nets` are networks some
/// container is attached to, and `nets` the networks people made.
pub fn sort(df: &Value, nets: &[String], used_nets: &HashSet<String>) -> Usage {
    // Docker 26 answers with arrays; API 1.52+ nests them in *Usage.Items.
    let list = |old: &str, new: &str| -> Vec<Value> {
        df[old].as_array().cloned().or_else(|| df[new]["Items"].as_array().cloned()).unwrap_or_default()
    };
    let mut u = Usage::default();

    let images = list("Images", "ImageUsage");
    let layers = df["LayersSize"].as_f64().or_else(|| df["ImageUsage"]["TotalSize"].as_f64());
    let mut all = 0.0;
    let mut biggest = Vec::new();
    for i in &images {
        let tags = tags(i);
        let size = num(&i["Size"]);
        let shared = i["SharedSize"].as_f64().filter(|s| *s >= 0.0).unwrap_or(0.0);
        let own = (size - shared).max(0.0);
        let used = i["Containers"].as_i64().unwrap_or(0) > 0;
        all += own;
        u.images.n += 1;
        if is_distro(&tags) {
            u.distros.add(size);
            if !used {
                u.distros_unused.add(own);
            }
        } else if !used && tags.is_empty() {
            u.dangling.add(own);
        } else if !used {
            u.images_unused.add(own);
        }
        let name = tags.first().cloned().unwrap_or_else(|| {
            let id = i["Id"].as_str().unwrap_or("").trim_start_matches("sha256:");
            format!("<none>@{}", &id[..id.len().min(12)])
        });
        biggest.push(Item { name, bytes: size, used });
    }
    u.images.bytes = layers.unwrap_or(all);
    biggest.sort_by(|a, b| b.bytes.total_cmp(&a.bytes));
    biggest.truncate(6);
    u.biggest_images = biggest;

    for c in list("Containers", "ContainerUsage") {
        let rw = num(&c["SizeRw"]);
        let running = c["State"].as_str() == Some("running");
        if has_label(&c, machines::LABEL) {
            u.machines.add(rw);
        } else if has_label(&c, "io.kubernetes.pod.name") {
            u.pods.add(rw);
        } else if !kept(&c) {
            u.containers.add(rw);
            if running {
                u.running += 1;
            } else {
                u.stopped.add(rw);
            }
        }
    }

    let mut vols = Vec::new();
    for v in list("Volumes", "VolumeUsage") {
        let size = num(&v["UsageData"]["Size"]);
        let used = v["UsageData"]["RefCount"].as_i64().unwrap_or(1) > 0;
        u.volumes.add(size);
        if !used {
            u.volumes_unused.add(size);
        }
        let name = v["Name"].as_str().unwrap_or("").to_string();
        if size >= 0.0 {
            u.volume_bytes.push((name.clone(), size));
        }
        vols.push(Item { name, bytes: size.max(0.0), used });
    }
    vols.sort_by(|a, b| b.bytes.total_cmp(&a.bytes));
    vols.truncate(6);
    u.biggest_volumes = vols;

    for b in list("BuildCache", "BuildCacheUsage") {
        let size = num(&b["Size"]);
        u.cache.add(size);
        if !b["InUse"].as_bool().unwrap_or(false) && !b["Shared"].as_bool().unwrap_or(false) {
            u.cache_unused.add(size);
        }
    }

    u.networks = nets.len();
    u.networks_unused = nets.iter().filter(|n| !used_nets.contains(*n)).count();
    u
}

/// GET a Docker API path through the engine's socket, as JSON.
async fn get(path: &str) -> Result<Value, String> {
    let mut s = tokio::net::UnixStream::connect(paths::docker_sock()).await.map_err(|e| e.to_string())?;
    s.write_all(format!("GET {path} HTTP/1.0\r\nHost: docker\r\n\r\n").as_bytes()).await.map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("The engine sent no answer.")?;
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return Err(head.lines().next().unwrap_or("").to_string());
    }
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// Networks people made (not bridge, host or none), and those in use.
async fn networks(d: &Docker) -> (Vec<String>, HashSet<String>) {
    let nets = d
        .list_networks(None)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|n| n.name)
        .filter(|n| !matches!(n.as_str(), "bridge" | "host" | "none"))
        .collect();
    let used = d
        .list_containers(Some(ListContainersOptionsBuilder::new().all(true).build()))
        .await
        .unwrap_or_default()
        .into_iter()
        .flat_map(|c| c.network_settings.and_then(|n| n.networks).unwrap_or_default().into_keys())
        .collect();
    (nets, used)
}

pub async fn usage(d: &Docker) -> Result<Usage, String> {
    let df = get("/system/df").await?;
    let (nets, used) = networks(d).await;
    Ok(sort(&df, &nets, &used))
}

fn err(e: bollard::errors::Error) -> String {
    match e {
        bollard::errors::Error::DockerResponseServerError { message, .. } => message,
        other => other.to_string(),
    }
}

/// Clean up what `sel` names. Returns what it freed, as a sentence.
pub async fn clean(d: &Docker, sel: Sel) -> Result<String, String> {
    let before = usage(d).await?.total();
    let mut removed: Vec<String> = Vec::new();
    let say = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });

    if sel.stopped {
        let list = d.list_containers(Some(ListContainersOptionsBuilder::new().all(true).build())).await.map_err(err)?;
        let mut n = 0;
        for c in list {
            let v = serde_json::json!({ "Labels": c.labels.clone().unwrap_or_default() });
            let running = matches!(c.state, Some(bollard::models::ContainerSummaryStateEnum::RUNNING) | Some(bollard::models::ContainerSummaryStateEnum::PAUSED) | Some(bollard::models::ContainerSummaryStateEnum::RESTARTING));
            let Some(id) = c.id else { continue };
            if running || kept(&v) {
                continue;
            }
            if d.remove_container(&id, Some(RemoveContainerOptionsBuilder::new().force(true).build())).await.is_ok() {
                n += 1;
            }
        }
        if n > 0 {
            removed.push(say(n, "container", "containers"));
        }
    }

    if sel.networks {
        if let Ok(r) = d.prune_networks(None::<bollard::query_parameters::PruneNetworksOptions>).await {
            let n = r.networks_deleted.map_or(0, |v| v.len());
            if n > 0 {
                removed.push(say(n, "network", "networks"));
            }
        }
    }

    if sel.volumes {
        let df = get("/system/df").await?;
        let vols = df["Volumes"].as_array().cloned().or_else(|| df["VolumeUsage"]["Items"].as_array().cloned()).unwrap_or_default();
        let mut n = 0;
        for v in vols.iter().filter(|v| v["UsageData"]["RefCount"].as_i64() == Some(0)) {
            if let Some(name) = v["Name"].as_str() {
                if d.remove_volume(name, None::<bollard::query_parameters::RemoveVolumeOptions>).await.is_ok() {
                    n += 1;
                }
            }
        }
        if n > 0 {
            removed.push(say(n, "volume", "volumes"));
        }
    }

    if sel.dangling || sel.images || sel.distros {
        // Images no container uses, after the containers above are gone.
        let containers = d.list_containers(Some(ListContainersOptionsBuilder::new().all(true).build())).await.map_err(err)?;
        let used: HashSet<String> = containers.into_iter().filter_map(|c| c.image_id).collect();
        let images = d.list_images(Some(ListImagesOptionsBuilder::new().all(false).build())).await.map_err(err)?;
        let mut n = 0;
        for i in images.iter().filter(|i| !used.contains(&i.id)) {
            let tags: Vec<String> = i.repo_tags.iter().filter(|t| *t != "<none>:<none>").cloned().collect();
            let pick = if is_distro(&tags) {
                sel.distros
            } else if tags.is_empty() {
                sel.dangling
            } else {
                sel.images
            };
            // No container uses it, so force only drops its other tags.
            if pick && d.remove_image(&i.id, Some(RemoveImageOptionsBuilder::new().force(true).build()), None).await.is_ok() {
                n += 1;
            }
        }
        if sel.dangling {
            let f = std::collections::HashMap::from([("dangling", vec!["true"])]);
            if let Ok(r) = d.prune_images(Some(PruneImagesOptionsBuilder::new().filters(&f).build())).await {
                n += r.images_deleted.map_or(0, |v| v.iter().filter(|x| x.deleted.is_some()).count());
            }
        }
        if n > 0 {
            removed.push(say(n, "image", "images"));
        }
    }

    if sel.cache {
        if let Ok(r) = d.prune_build(Some(PruneBuildOptionsBuilder::new().all(true).build())).await {
            if r.caches_deleted.is_some_and(|v| !v.is_empty()) {
                removed.push("build cache".into());
            }
        }
    }

    let after = usage(d).await.map(|u| u.total()).unwrap_or(before);
    let freed = (before - after).max(0.0);
    Ok(if removed.is_empty() {
        "Nothing to clean up".to_string()
    } else {
        format!("Freed {}: {}", fmt_bytes(freed), removed.join(", "))
    })
}

/// "1.2 GB", "340 MB", "12 KB".
pub fn fmt_bytes(b: f64) -> String {
    let b = b.max(0.0);
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{b:.0} B")
    }
}

/// How much of the engine's disk image macOS has actually allocated.
pub fn disk_image_bytes() -> f64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(paths::disk()).map(|m| m.blocks() as f64 * 512.0).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn df() -> Value {
        json!({
            "LayersSize": 3_000_000_000u64,
            "Images": [
                { "Id": "sha256:aaa", "RepoTags": ["nginx:1.27"], "Size": 200_000_000, "SharedSize": 50_000_000, "Containers": 1 },
                { "Id": "sha256:bbb", "RepoTags": ["redis:7"], "Size": 100_000_000, "SharedSize": 0, "Containers": 0 },
                { "Id": "sha256:ccc", "RepoTags": ["<none>:<none>"], "Size": 30_000_000, "SharedSize": 0, "Containers": 0 },
                { "Id": "sha256:ddd", "RepoTags": ["patina.local/ubuntu:24.04"], "Size": 144_000_000, "SharedSize": 100_000_000, "Containers": 0 },
                { "Id": "sha256:eee", "RepoTags": ["patina.local/debian:13"], "Size": 206_000_000, "SharedSize": 0, "Containers": 1 }
            ],
            "Containers": [
                { "Id": "1", "State": "running", "SizeRw": 1_000_000, "Labels": {} },
                { "Id": "2", "State": "exited", "SizeRw": 5_000_000, "Labels": { "com.docker.compose.project": "x" } },
                { "Id": "3", "State": "exited", "SizeRw": 90_000_000, "Labels": { "dev.patina.machine": "debian" } },
                { "Id": "4", "State": "exited", "SizeRw": 2_000_000, "Labels": { "io.kubernetes.pod.name": "coredns" } },
                { "Id": "5", "State": "exited", "Labels": { "dev.patina.helper": "vm" } }
            ],
            "Volumes": [
                { "Name": "db", "UsageData": { "Size": 400_000_000, "RefCount": 1 } },
                { "Name": "old", "UsageData": { "Size": 70_000_000, "RefCount": 0 } }
            ],
            "BuildCache": [
                { "ID": "a", "Size": 10_000_000, "InUse": false, "Shared": false },
                { "ID": "b", "Size": 20_000_000, "InUse": true, "Shared": false }
            ]
        })
    }

    #[test]
    fn sorts_disk_usage() {
        let nets = vec!["app_default".to_string(), "spare".to_string()];
        let used: HashSet<String> = ["app_default".to_string(), "bridge".to_string()].into();
        let u = sort(&df(), &nets, &used);
        assert_eq!(u.images, Part { n: 5, bytes: 3e9 });
        assert_eq!(u.images_unused, Part { n: 1, bytes: 1e8 });
        assert_eq!(u.dangling, Part { n: 1, bytes: 3e7 });
        assert_eq!(u.distros.n, 2);
        assert_eq!(u.distros_unused, Part { n: 1, bytes: 4.4e7 });
        assert_eq!((u.containers.n, u.running, u.stopped), (2, 1, Part { n: 1, bytes: 5e6 }));
        assert_eq!(u.machines, Part { n: 1, bytes: 9e7 });
        assert_eq!(u.pods.n, 1);
        assert_eq!(u.volumes_unused, Part { n: 1, bytes: 7e7 });
        assert_eq!(u.cache_unused, Part { n: 1, bytes: 1e7 });
        assert_eq!((u.networks, u.networks_unused), (2, 1));
        assert_eq!(u.biggest_images[0].name, "patina.local/debian:13");
        assert_eq!(u.biggest_volumes[0], Item { name: "db".into(), bytes: 4e8, used: true });
        // Volumes are left out of what Clean up offers by default.
        assert_eq!(u.reclaimable(), 5e6 + 3e7 + 1e8 + 4.4e7 + 1e7);
        assert_eq!(u.freeable(&Sel::all()), u.reclaimable() + 7e7);
    }

    #[test]
    fn reads_the_newer_shape_too() {
        let d = df();
        let newer = json!({
            "ImageUsage": { "TotalSize": 3_000_000_000u64, "Items": d["Images"] },
            "ContainerUsage": { "Items": d["Containers"] },
            "VolumeUsage": { "Items": d["Volumes"] },
            "BuildCacheUsage": { "Items": d["BuildCache"] }
        });
        assert_eq!(sort(&newer, &[], &HashSet::new()), sort(&d, &[], &HashSet::new()));
    }

    #[test]
    fn keeps_machines_pods_and_helpers() {
        for labels in [json!({ "dev.patina.machine": "x" }), json!({ "io.kubernetes.pod.name": "x" }), json!({ "dev.patina.helper": "vm" })] {
            assert!(kept(&json!({ "Labels": labels })));
        }
        assert!(!kept(&json!({ "Labels": { "com.docker.compose.project": "x" } })));
    }

    /// Reads the running engine: `cargo test usage_from_the_engine -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn usage_from_the_engine() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let u = rt.block_on(async {
            let d = Docker::connect_with_unix(&paths::docker_sock().to_string_lossy(), 60, bollard::API_DEFAULT_VERSION).unwrap();
            usage(&d).await.unwrap()
        });
        let p = |n: &str, x: Part| eprintln!("{n:<16} {:>4}  {:>9}", x.n, fmt_bytes(x.bytes));
        p("images", u.images);
        p("  unused", u.images_unused);
        p("  dangling", u.dangling);
        p("  distros", u.distros);
        p("  distros unused", u.distros_unused);
        p("containers", u.containers);
        p("  stopped", u.stopped);
        p("pods", u.pods);
        p("machines", u.machines);
        p("volumes", u.volumes);
        p("  unused", u.volumes_unused);
        p("build cache", u.cache);
        p("  unused", u.cache_unused);
        eprintln!("networks {} ({} unused), running {}", u.networks, u.networks_unused, u.running);
        eprintln!("total {}  reclaimable {}  disk image {}", fmt_bytes(u.total()), fmt_bytes(u.reclaimable()), fmt_bytes(disk_image_bytes()));
        assert!(u.images.n > 0);
    }

    #[test]
    fn writes_sizes() {
        assert_eq!(fmt_bytes(2_340_000_000.0), "2.3 GB");
        assert_eq!(fmt_bytes(340_000_000.0), "340 MB");
        assert_eq!(fmt_bytes(12_000.0), "12 KB");
        assert_eq!(fmt_bytes(-5.0), "0 B");
    }
}
