//! Kubernetes: a single-node k3s cluster inside Patina's VM.
//!
//! k3s runs as a systemd service in the guest with `--docker`, so pods are
//! ordinary Docker containers on Patina's engine and images built with
//! `docker build` run in the cluster without a registry. Traefik and the
//! service load balancer are left out: their host ports would take over
//! ports that published containers use.
//!
//! The app reaches the guest through the Docker API: a short-lived
//! privileged container enters the VM's namespaces (`nsenter`) and runs a
//! script there. The API server is forwarded to `localhost:6443` by the
//! engine while `kubernetes` is on in the config, and the cluster is merged
//! into `~/.kube/config` as the `patina` context, by the guest's kubectl
//! through the shared home folder.

use std::collections::{BTreeMap, HashMap};

use bollard::models::{ContainerCreateBody, ContainerSummary, HostConfig};
use bollard::query_parameters::{CreateImageOptionsBuilder, ListContainersOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptions};
use bollard::Docker;
use futures_util::StreamExt;
use serde_json::Value;

use super::{config, paths};

/// Small image with `nsenter` (busybox) for running scripts in the VM.
const HELPER: &str = "alpine:3.20";
/// Label on Patina's own short-lived containers, which the UI hides.
pub const HELPER_LABEL: &str = "dev.patina.helper";
/// Where Patina forwards the API server on the Mac.
pub const API_PORT: u16 = 6443;

/// Run `script` with `sh` as root inside the VM. Returns its output, or
/// the last thing it printed when it fails.
pub async fn in_vm(d: &Docker, script: &str) -> Result<String, String> {
    if d.inspect_image(HELPER).await.is_err() {
        let (img, tag) = HELPER.split_once(':').unwrap_or((HELPER, "latest"));
        let mut pull = d.create_image(Some(CreateImageOptionsBuilder::new().from_image(img).tag(tag).build()), None, None);
        while let Some(r) = pull.next().await {
            r.map_err(|e| format!("Couldn't pull {HELPER}: {e}"))?;
        }
    }
    let body = ContainerCreateBody {
        image: Some(HELPER.into()),
        entrypoint: Some(vec!["nsenter".into()]),
        cmd: Some(["-t", "1", "-m", "-u", "-n", "-i", "-p", "--", "sh", "-c", script].map(String::from).to_vec()),
        labels: Some(HashMap::from([(HELPER_LABEL.to_string(), "vm".to_string())])),
        host_config: Some(HostConfig { privileged: Some(true), pid_mode: Some("host".into()), ..Default::default() }),
        ..Default::default()
    };
    let id = d.create_container(None::<bollard::query_parameters::CreateContainerOptions>, body).await.map_err(|e| e.to_string())?.id;
    let r = run_helper(d, &id).await;
    let _ = d.remove_container(&id, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
    r
}

async fn run_helper(d: &Docker, id: &str) -> Result<String, String> {
    d.start_container(id, None).await.map_err(|e| e.to_string())?;
    let mut code = 0;
    let mut wait = d.wait_container(id, None);
    while let Some(r) = wait.next().await {
        match r {
            Ok(w) => code = w.status_code,
            Err(bollard::errors::Error::DockerContainerWaitError { code: c, .. }) => code = c,
            Err(e) => return Err(e.to_string()),
        }
    }
    let mut text = String::new();
    let mut logs = d.logs(id, Some(LogsOptionsBuilder::new().stdout(true).stderr(true).build()));
    while let Some(Ok(chunk)) = logs.next().await {
        text.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
    }
    if code == 0 {
        Ok(text)
    } else {
        let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        Err(if last.is_empty() { format!("The script in the VM failed with code {code}.") } else { last.to_string() })
    }
}

/// Install k3s on first use, start it, wait for the API, and merge the
/// cluster into `~/.kube/config` as the current `patina` context.
fn up_script(home: &str) -> String {
    format!(
        r#"set -e
if [ ! -x /usr/local/bin/k3s ]; then
  curl -sfL https://get.k3s.io -o /tmp/patina-k3s-install.sh || {{ echo "Couldn't download k3s. Check the connection, then try again."; exit 1; }}
  INSTALL_K3S_SKIP_START=true INSTALL_K3S_EXEC="server --docker --disable=traefik --disable=servicelb --write-kubeconfig-mode=600 --node-name=patina" sh /tmp/patina-k3s-install.sh >/tmp/patina-k3s-install.log 2>&1 || {{ tail -n 3 /tmp/patina-k3s-install.log; echo "Couldn't install k3s."; exit 1; }}
  rm -f /tmp/patina-k3s-install.sh
fi
systemctl enable --now k3s
i=0
until k3s kubectl get --raw /readyz >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -ge 240 ]; then echo "The cluster didn't start within 4 minutes."; exit 1; fi
  sleep 1
done
{creds}
mkdir -p '{home}/.kube'
cfg='{home}/.kube/config'
if [ -s "$cfg" ]; then
  KUBECONFIG="/tmp/patina-kube.yaml:$cfg" k3s kubectl config view --raw > /tmp/patina-kube-merged.yaml
  cat /tmp/patina-kube-merged.yaml > "$cfg"
else
  cat /tmp/patina-kube.yaml > "$cfg"
fi
chmod 600 "$cfg"
rm -f /tmp/patina-kube.yaml /tmp/patina-kube-merged.yaml
"#,
        creds = creds_script(home)
    )
}

/// The cluster's kubeconfig with its names changed to `patina`, in
/// /tmp/patina-kube.yaml, and as JSON in `~/.patina/kube.json` for the app.
fn creds_script(home: &str) -> String {
    format!(
        r#"sed -E 's/^( *(- )?(name|cluster|user|current-context): )default$/\1patina/' /etc/rancher/k3s/k3s.yaml > /tmp/patina-kube.yaml
k3s kubectl config view --raw -o json --kubeconfig=/tmp/patina-kube.yaml > '{home}/.patina/kube.json'
chmod 600 '{home}/.patina/kube.json'"#
    )
}

fn quoted_home() -> String {
    paths::home().display().to_string().replace('\'', "'\\''")
}

/// Write `~/.patina/kube.json` if it's missing, e.g. for a cluster turned
/// on by an older Patina.
pub async fn ensure_creds(d: &Docker) -> Result<(), String> {
    if paths::kube_creds().exists() {
        return Ok(());
    }
    in_vm(d, &format!("set -e\n{}\nrm -f /tmp/patina-kube.yaml", creds_script(&quoted_home()))).await.map(|_| ())
}

/// Stop k3s and keep it from starting with the VM.
const STOP: &str = "systemctl disable --now k3s >/dev/null 2>&1 || true";
/// Unmount pod volumes and drop the cluster's network rules and links.
const CLEAN: &str = "[ ! -x /usr/local/bin/k3s-killall.sh ] || /usr/local/bin/k3s-killall.sh >/dev/null 2>&1 || true";
/// Forget the cluster's state; the next start makes a new one.
const WIPE: &str = "rm -rf /var/lib/rancher/k3s/server /var/lib/rancher/k3s/agent /etc/rancher/k3s/k3s.yaml /var/lib/kubelet";

fn set_enabled(on: bool) -> Result<(), String> {
    let mut c = config::load();
    if c.kubernetes != on {
        c.kubernetes = on;
        config::save(&c)?;
    }
    Ok(())
}

/// Turn the cluster on. On failure it's turned off again.
pub async fn up(d: &Docker) -> Result<String, String> {
    set_enabled(true)?;
    match in_vm(d, &up_script(&quoted_home())).await {
        Ok(_) => Ok("Kubernetes is on. kubectl uses the patina context.".into()),
        Err(e) => {
            let _ = down(d, false).await;
            Err(e)
        }
    }
}

/// Turn the cluster off, removing its pods' containers. With `wipe`, its
/// state goes too.
pub async fn down(d: &Docker, wipe: bool) -> Result<(), String> {
    set_enabled(false)?;
    in_vm(d, STOP).await?;
    let pods = d.list_containers(Some(ListContainersOptionsBuilder::new().all(true).build())).await.map_err(|e| e.to_string())?;
    for c in pods.iter().filter(|c| is_pod(c)) {
        if let Some(id) = &c.id {
            let _ = d.remove_container(id, Some(RemoveContainerOptions { force: true, v: true, ..Default::default() })).await;
        }
    }
    in_vm(d, &if wipe { format!("{CLEAN}; {WIPE}") } else { CLEAN.to_string() }).await?;
    if wipe {
        let _ = std::fs::remove_file(paths::kube_creds());
    }
    Ok(())
}

fn label<'a>(c: &'a ContainerSummary, key: &str) -> Option<&'a str> {
    c.labels.as_ref()?.get(key).map(String::as_str)
}

/// A container that belongs to a Kubernetes pod.
pub fn is_pod(c: &ContainerSummary) -> bool {
    label(c, "io.kubernetes.pod.name").is_some()
}

/// A container the UI shouldn't list: pods' and Patina's own helpers.
pub fn hidden(c: &ContainerSummary) -> bool {
    is_pod(c) || label(c, HELPER_LABEL).is_some()
}

#[derive(Debug, PartialEq)]
pub struct Pod {
    /// The pod's name, with its namespace unless that's kube-system.
    pub id: String,
    pub ready: String,
    pub phase: String,
    /// Running, or finished cleanly.
    pub ok: bool,
    /// The Docker container of its newest app container, for logs.
    pub container: String,
}

/// Container name -> (created, running, exit code, Docker name) of its newest container.
type Containers = HashMap<String, (i64, bool, i64, String)>;

/// Pods, read from their containers' labels: kube-system first.
pub fn pods(list: &[ContainerSummary]) -> Vec<Pod> {
    // (not kube-system, namespace, pod) -> its containers
    let mut by_pod: BTreeMap<(bool, String, String), Containers> = BTreeMap::new();
    for c in list {
        let (Some(ns), Some(pod)) = (label(c, "io.kubernetes.pod.namespace"), label(c, "io.kubernetes.pod.name")) else { continue };
        let containers = by_pod.entry((ns != "kube-system", ns.to_string(), pod.to_string())).or_default();
        if label(c, "io.kubernetes.docker.type") == Some("podsandbox") {
            continue;
        }
        let Some(name) = label(c, "io.kubernetes.container.name") else { continue };
        let running = c.state.is_some_and(|s| s.to_string() == "running");
        let status = c.status.as_deref().unwrap_or("");
        let code = status.split_once('(').and_then(|(_, r)| r.split_once(')')).and_then(|(n, _)| n.trim().parse().ok()).unwrap_or(0);
        let created = c.created.unwrap_or(0);
        let docker = c.names.as_ref().and_then(|n| n.first()).map(|n| n.trim_start_matches('/').to_string()).unwrap_or_default();
        // Restarts leave the old container behind; the newest one counts.
        let e = containers.entry(name.to_string()).or_insert((created, running, code, docker.clone()));
        if created >= e.0 {
            *e = (created, running, code, docker);
        }
    }
    by_pod
        .into_iter()
        .map(|((_, ns, pod), cs)| {
            let total = cs.len();
            let running = cs.values().filter(|c| c.1).count();
            let phase = if total == 0 {
                "ContainerCreating"
            } else if running == total {
                "Running"
            } else if running == 0 && cs.values().all(|c| c.2 == 0) {
                "Completed"
            } else if cs.values().any(|c| !c.1 && c.2 != 0) {
                "Error"
            } else {
                "ContainerCreating"
            };
            let container = cs.values().max_by_key(|c| (c.1, c.0)).map(|c| c.3.clone()).unwrap_or_default();
            Pod {
                id: if ns == "kube-system" { pod } else { format!("{ns}/{pod}") },
                ready: format!("{running}/{}", total.max(1)),
                phase: phase.into(),
                ok: matches!(phase, "Running" | "Completed"),
                container,
            }
        })
        .collect()
}

// --------------------------------------------------------------- the API

/// How an object is doing, for its status dot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Good,
    Busy,
    Bad,
    Idle,
}

/// One row of a `kubectl get`-style table.
#[derive(Clone, Debug)]
pub struct Obj {
    pub kind: &'static str,
    pub ns: String,
    pub name: String,
    pub health: Health,
    /// The kind's columns, already formatted.
    pub cols: Vec<String>,
    /// Unix seconds.
    pub created: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub version: String,
    /// Millicores in use and allocatable; use is `None` until metrics arrive.
    pub cpu_m: Option<f64>,
    pub cpu_cap_m: f64,
    /// Bytes in use and allocatable.
    pub mem: Option<f64>,
    pub mem_cap: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub objs: Vec<Obj>,
    pub node: Option<Node>,
}

/// An HTTPS client for the cluster's API on localhost, with the credentials
/// in `~/.patina/kube.json`.
pub struct Api {
    agent: ureq::Agent,
    server: String,
}

fn pem(v: &Value) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(v.as_str().unwrap_or("")).map_err(|e| e.to_string())
}

impl Api {
    pub fn load() -> Result<Api, String> {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        let text = std::fs::read_to_string(paths::kube_creds()).map_err(|_| "Patina doesn't have the cluster's credentials yet.".to_string())?;
        let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let cluster = &v["clusters"][0]["cluster"];
        let user = &v["users"][0]["user"];
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(&pem(&cluster["certificate-authority-data"])?) {
            roots.add(c.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        }
        let certs = CertificateDer::pem_slice_iter(&pem(&user["client-certificate-data"])?).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        let key = PrivateKeyDer::from_pem_slice(&pem(&user["client-key-data"])?).map_err(|e| e.to_string())?;
        let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_client_auth_cert(certs, key)
            .map_err(|e| e.to_string())?;
        let agent = ureq::AgentBuilder::new().tls_config(std::sync::Arc::new(tls)).timeout(std::time::Duration::from_secs(8)).build();
        let server = cluster["server"].as_str().unwrap_or("https://127.0.0.1:6443").trim_end_matches('/').to_string();
        Ok(Api { agent, server })
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        let r = self.agent.get(&format!("{}{path}", self.server)).call().map_err(|e| self.fail(e))?;
        serde_json::from_str(&r.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    /// The cluster's own message for a refusal (a Status object), or why it
    /// couldn't be reached.
    fn fail(&self, e: ureq::Error) -> String {
        match e {
            ureq::Error::Status(code, r) => {
                let msg = r.into_string().ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["message"].as_str().map(str::to_string));
                match msg {
                    Some(m) => {
                        let mut c = m.chars();
                        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or(m) + "."
                    }
                    None => format!("The cluster refused with {code}."),
                }
            }
            _ => format!("Can't reach the cluster's API on {}.", self.server.trim_start_matches("https://")),
        }
    }

    /// Delete an object, and what it owns (pods of a deployment, say).
    pub fn delete(&self, kind: &str, ns: &str, name: &str) -> Result<(), String> {
        let path = object_path(kind, ns, name).ok_or_else(|| format!("Patina can't delete a {kind}."))?;
        self.agent.delete(&format!("{}{path}?propagationPolicy=Background", self.server)).call().map_err(|e| self.fail(e))?;
        Ok(())
    }

    /// Set how many replicas a deployment or stateful set runs.
    pub fn scale(&self, kind: &str, ns: &str, name: &str, replicas: u32) -> Result<(), String> {
        let path = object_path(kind, ns, name).filter(|_| can_scale(kind)).ok_or_else(|| format!("A {kind} can't be scaled."))?;
        self.agent
            .request("PATCH", &format!("{}{path}/scale", self.server))
            .set("Content-Type", "application/merge-patch+json")
            .send_string(&serde_json::json!({ "spec": { "replicas": replicas } }).to_string())
            .map_err(|e| self.fail(e))?;
        Ok(())
    }

    fn items(&self, path: &str) -> Result<Vec<Value>, String> {
        Ok(self.get(path)?["items"].as_array().cloned().unwrap_or_default())
    }
}

/// Where a kind lives in the API: its group's path, plural, and whether
/// objects belong to a namespace.
fn resource(kind: &str) -> Option<(&'static str, &'static str, bool)> {
    Some(match kind {
        "Pod" => ("/api/v1", "pods", true),
        "Service" => ("/api/v1", "services", true),
        "PersistentVolumeClaim" => ("/api/v1", "persistentvolumeclaims", true),
        "PersistentVolume" => ("/api/v1", "persistentvolumes", false),
        "ConfigMap" => ("/api/v1", "configmaps", true),
        "Secret" => ("/api/v1", "secrets", true),
        "Deployment" => ("/apis/apps/v1", "deployments", true),
        "StatefulSet" => ("/apis/apps/v1", "statefulsets", true),
        "DaemonSet" => ("/apis/apps/v1", "daemonsets", true),
        "Job" => ("/apis/batch/v1", "jobs", true),
        "CronJob" => ("/apis/batch/v1", "cronjobs", true),
        "Ingress" => ("/apis/networking.k8s.io/v1", "ingresses", true),
        _ => return None,
    })
}

fn object_path(kind: &str, ns: &str, name: &str) -> Option<String> {
    let (base, plural, namespaced) = resource(kind)?;
    Some(if namespaced { format!("{base}/namespaces/{ns}/{plural}/{name}") } else { format!("{base}/{plural}/{name}") })
}

pub fn can_scale(kind: &str) -> bool {
    matches!(kind, "Deployment" | "StatefulSet")
}

/// The kinds the browser lists besides pods, in the order it fetches them.
const KINDS: [&str; 11] =
    ["Deployment", "StatefulSet", "DaemonSet", "Job", "CronJob", "Service", "Ingress", "PersistentVolumeClaim", "PersistentVolume", "ConfigMap", "Secret"];

/// Everything the browser shows, read from the API in one go.
pub fn snapshot(api: &Api) -> Result<Snapshot, String> {
    let mut objs = Vec::new();
    // Metrics come from metrics-server, which may not be ready yet.
    let usage: HashMap<(String, String), (f64, f64)> = api
        .items("/apis/metrics.k8s.io/v1beta1/pods")
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let (cpu, mem) = m["containers"].as_array().into_iter().flatten().fold((0.0, 0.0), |(c, b), x| (c + cpu_m(&x["usage"]["cpu"]), b + bytes(&x["usage"]["memory"])));
            ((s(&m["metadata"]["namespace"]), s(&m["metadata"]["name"])), (cpu, mem))
        })
        .collect();
    for p in api.items("/api/v1/pods")? {
        let key = (s(&p["metadata"]["namespace"]), s(&p["metadata"]["name"]));
        objs.push(pod(&p, usage.get(&key).copied()));
    }
    for kind in KINDS {
        let (base, plural, _) = resource(kind).unwrap_or_default();
        objs.extend(api.items(&format!("{base}/{plural}"))?.iter().filter_map(|o| other(kind, o)));
    }
    let node = api.items("/api/v1/nodes")?.first().map(|n| {
        let name = s(&n["metadata"]["name"]);
        let m = api.get(&format!("/apis/metrics.k8s.io/v1beta1/nodes/{name}")).ok();
        Node {
            version: s(&n["status"]["nodeInfo"]["kubeletVersion"]),
            cpu_m: m.as_ref().map(|m| cpu_m(&m["usage"]["cpu"])),
            cpu_cap_m: cpu_m(&n["status"]["allocatable"]["cpu"]),
            mem: m.as_ref().map(|m| bytes(&m["usage"]["memory"])),
            mem_cap: bytes(&n["status"]["allocatable"]["memory"]),
        }
    });
    Ok(Snapshot { objs, node })
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn created(o: &Value) -> i64 {
    o["metadata"]["creationTimestamp"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map_or(0, |t| t.timestamp())
}

fn n(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

/// A CPU quantity in millicores: "250m", "2", "123456789n".
pub fn cpu_m(q: &Value) -> f64 {
    let q = q.as_str().unwrap_or("0");
    let (num, scale) = match q.char_indices().last() {
        Some((i, 'n')) => (&q[..i], 1e-6),
        Some((i, 'u')) => (&q[..i], 1e-3),
        Some((i, 'm')) => (&q[..i], 1.0),
        _ => (q, 1000.0),
    };
    num.parse::<f64>().unwrap_or(0.0) * scale
}

/// A memory quantity in bytes: "48Mi", "1Gi", "500M", "1024".
pub fn bytes(q: &Value) -> f64 {
    let q = q.as_str().unwrap_or("0");
    let units = [("Ki", 1024f64), ("Mi", 1024f64.powi(2)), ("Gi", 1024f64.powi(3)), ("Ti", 1024f64.powi(4)), ("k", 1e3), ("K", 1e3), ("M", 1e6), ("G", 1e9), ("T", 1e12)];
    for (u, m) in units {
        if let Some(num) = q.strip_suffix(u) {
            return num.parse::<f64>().unwrap_or(0.0) * m;
        }
    }
    q.parse().unwrap_or(0.0)
}

pub fn fmt_cpu(m: f64) -> String {
    if m >= 1000.0 && m % 1000.0 == 0.0 {
        format!("{}", (m / 1000.0) as i64)
    } else if m >= 1000.0 {
        format!("{:.1}", m / 1000.0)
    } else {
        format!("{}m", m.round() as i64)
    }
}

pub fn fmt_bytes(b: f64) -> String {
    let mi = b / 1024f64.powi(2);
    if mi >= 1024.0 {
        format!("{:.1}Gi", mi / 1024.0)
    } else {
        format!("{}Mi", mi.round() as i64)
    }
}

/// A pod row: READY, STATUS, RESTARTS, CPU, MEMORY, with the status worded
/// the way kubectl words it.
fn pod(p: &Value, usage: Option<(f64, f64)>) -> Obj {
    let statuses = p["status"]["containerStatuses"].as_array().cloned().unwrap_or_default();
    let total = p["spec"]["containers"].as_array().map_or(statuses.len(), Vec::len);
    let ready = statuses.iter().filter(|c| c["ready"].as_bool() == Some(true)).count();
    let restarts: i64 = statuses.iter().map(|c| n(&c["restartCount"])).sum();
    let phase = s(&p["status"]["phase"]);
    let reason = statuses.iter().find_map(|c| c["state"]["waiting"]["reason"].as_str().or_else(|| c["state"]["terminated"]["reason"].as_str()).map(str::to_string));
    let status = if !p["metadata"]["deletionTimestamp"].is_null() {
        "Terminating".to_string()
    } else if let Some(r) = reason.filter(|_| phase != "Succeeded" || ready == 0) {
        r
    } else if phase.is_empty() {
        "Pending".into()
    } else {
        phase.clone()
    };
    let health = match status.as_str() {
        "Running" if ready == total => Health::Good,
        "Succeeded" | "Completed" => Health::Idle,
        "Running" | "Pending" | "ContainerCreating" | "PodInitializing" | "Terminating" => Health::Busy,
        _ => Health::Bad,
    };
    let (cpu, mem) = usage.map_or(("—".to_string(), "—".to_string()), |(c, b)| (fmt_cpu(c), fmt_bytes(b)));
    Obj {
        kind: "Pod",
        ns: s(&p["metadata"]["namespace"]),
        name: s(&p["metadata"]["name"]),
        health,
        cols: vec![format!("{ready}/{total}"), status, restarts.to_string(), cpu, mem],
        created: created(p),
    }
}

/// Rows for everything but pods. Workloads: KIND, READY. Services: TYPE,
/// CLUSTER-IP, PORTS. Storage: KIND, STATUS, CAPACITY, CLASS. Config:
/// KIND, DATA.
fn other(kind: &'static str, o: &Value) -> Option<Obj> {
    let name = s(&o["metadata"]["name"]);
    if kind == "ConfigMap" && name == "kube-root-ca.crt" {
        return None;
    }
    let spec = &o["spec"];
    let st = &o["status"];
    let ratio = |have: i64, want: i64| (format!("{have}/{want}"), if have >= want { if want == 0 { Health::Idle } else { Health::Good } } else { Health::Busy });
    let (cols, health) = match kind {
        "Deployment" | "StatefulSet" => {
            let (r, h) = ratio(n(&st["readyReplicas"]), spec["replicas"].as_i64().unwrap_or(1));
            (vec![kind.to_string(), r], h)
        }
        "DaemonSet" => {
            let (r, h) = ratio(n(&st["numberReady"]), n(&st["desiredNumberScheduled"]));
            (vec![kind.to_string(), r], h)
        }
        "Job" => {
            let want = spec["completions"].as_i64().unwrap_or(1);
            let done = n(&st["succeeded"]);
            let h = if n(&st["failed"]) > 0 && done < want { Health::Bad } else if done >= want { Health::Idle } else { Health::Busy };
            (vec![kind.to_string(), format!("{done}/{want}")], h)
        }
        "CronJob" => (vec![kind.to_string(), s(&spec["schedule"])], if spec["suspend"].as_bool() == Some(true) { Health::Idle } else { Health::Good }),
        "Service" => {
            let ports = spec["ports"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    let proto = p["protocol"].as_str().unwrap_or("TCP");
                    match p["nodePort"].as_i64() {
                        Some(np) => format!("{}:{np}/{proto}", n(&p["port"])),
                        None => format!("{}/{proto}", n(&p["port"])),
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            (vec![s(&spec["type"]), s(&spec["clusterIP"]), if ports.is_empty() { "<none>".into() } else { ports }], Health::Good)
        }
        "Ingress" => {
            let hosts = spec["rules"].as_array().into_iter().flatten().filter_map(|r| r["host"].as_str()).collect::<Vec<_>>().join(",");
            (vec!["Ingress".into(), String::new(), if hosts.is_empty() { "*".into() } else { hosts }], Health::Good)
        }
        "PersistentVolumeClaim" | "PersistentVolume" => {
            let phase = s(&st["phase"]);
            let cap = if kind == "PersistentVolume" { &spec["capacity"]["storage"] } else { &st["capacity"]["storage"] };
            let h = match phase.as_str() {
                "Bound" => Health::Good,
                "Pending" => Health::Busy,
                "Available" | "Released" => Health::Idle,
                _ => Health::Bad,
            };
            let short = if kind == "PersistentVolume" { "PV" } else { "PVC" };
            (vec![short.into(), phase, s(cap), s(&spec["storageClassName"])], h)
        }
        "ConfigMap" => {
            let keys = o["data"].as_object().map_or(0, |m| m.len()) + o["binaryData"].as_object().map_or(0, |m| m.len());
            (vec![kind.into(), keys.to_string()], Health::Good)
        }
        "Secret" => {
            // Only the count of keys is kept, never their values.
            let keys = o["data"].as_object().map_or(0, |m| m.len());
            (vec![kind.into(), format!("{keys} · {}", s(&o["type"]))], Health::Good)
        }
        _ => return None,
    };
    Some(Obj { kind, ns: s(&o["metadata"]["namespace"]), name, health, cols, created: created(o) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::ContainerSummaryStateEnum as S;

    fn c(ns: &str, pod: &str, name: Option<&str>, state: S, status: &str, created: i64) -> ContainerSummary {
        let mut labels = HashMap::from([("io.kubernetes.pod.namespace".to_string(), ns.to_string()), ("io.kubernetes.pod.name".to_string(), pod.to_string())]);
        match name {
            Some(n) => {
                labels.insert("io.kubernetes.container.name".into(), n.into());
                labels.insert("io.kubernetes.docker.type".into(), "container".into());
            }
            None => {
                labels.insert("io.kubernetes.docker.type".into(), "podsandbox".into());
            }
        }
        ContainerSummary { labels: Some(labels), state: Some(state), status: Some(status.into()), created: Some(created), ..Default::default() }
    }

    #[test]
    fn reads_pods_from_container_labels() {
        let list = vec![
            c("default", "web-5d9", None, S::RUNNING, "Up 1 minute", 1),
            c("default", "web-5d9", Some("web"), S::RUNNING, "Up 1 minute", 2),
            c("kube-system", "coredns-7b9", None, S::RUNNING, "Up 2 minutes", 1),
            // A restart: the old one exited, the new one runs.
            c("kube-system", "coredns-7b9", Some("coredns"), S::EXITED, "Exited (2) 1 minute ago", 2),
            c("kube-system", "coredns-7b9", Some("coredns"), S::RUNNING, "Up 1 minute", 3),
            c("kube-system", "metrics-server-6f4", None, S::RUNNING, "Up 5 seconds", 1),
            c("default", "job-x", Some("run"), S::EXITED, "Exited (0) 3 seconds ago", 1),
            c("default", "bad", Some("app"), S::EXITED, "Exited (1) 3 seconds ago", 1),
        ];
        let got: Vec<Pod> = pods(&list).into_iter().map(|p| Pod { container: String::new(), ..p }).collect();
        let p = |id: &str, ready: &str, phase: &str, ok: bool| Pod { id: id.into(), ready: ready.into(), phase: phase.into(), ok, container: String::new() };
        assert_eq!(
            got,
            [
                p("coredns-7b9", "1/1", "Running", true),
                p("metrics-server-6f4", "0/1", "ContainerCreating", false),
                p("default/bad", "0/1", "Error", false),
                p("default/job-x", "0/1", "Completed", true),
                p("default/web-5d9", "1/1", "Running", true),
            ]
        );
    }

    #[test]
    fn finds_objects_in_the_api() {
        assert_eq!(object_path("Deployment", "shop", "web").as_deref(), Some("/apis/apps/v1/namespaces/shop/deployments/web"));
        assert_eq!(object_path("Pod", "kube-system", "coredns-1").as_deref(), Some("/api/v1/namespaces/kube-system/pods/coredns-1"));
        assert_eq!(object_path("PersistentVolume", "", "pvc-1").as_deref(), Some("/api/v1/persistentvolumes/pvc-1"));
        assert_eq!(object_path("Node", "", "patina"), None);
        assert!(KINDS.iter().all(|k| resource(k).is_some()));
        assert!(can_scale("StatefulSet") && !can_scale("DaemonSet"));
    }

    #[test]
    fn reads_quantities() {
        let q = |s: &str| Value::String(s.into());
        assert_eq!(cpu_m(&q("250m")), 250.0);
        assert_eq!(cpu_m(&q("2")), 2000.0);
        assert!((cpu_m(&q("123456789n")) - 123.456789).abs() < 1e-6);
        assert_eq!(bytes(&q("48Mi")), 48.0 * 1024.0 * 1024.0);
        assert_eq!(bytes(&q("500M")), 5e8);
        assert_eq!(bytes(&q("1024")), 1024.0);
        assert_eq!(fmt_cpu(1500.0), "1.5");
        assert_eq!(fmt_cpu(4000.0), "4");
        assert_eq!(fmt_bytes(bytes(&q("1536Mi"))), "1.5Gi");
    }

    #[test]
    fn words_pods_like_kubectl() {
        let p: Value = serde_json::from_str(r#"{"metadata":{"name":"web-1","namespace":"shop","creationTimestamp":"2026-11-25T10:00:00Z"},
            "spec":{"containers":[{},{}]},
            "status":{"phase":"Running","containerStatuses":[
                {"ready":true,"restartCount":1,"state":{"running":{}}},
                {"ready":false,"restartCount":4,"state":{"waiting":{"reason":"CrashLoopBackOff"}}}]}}"#).unwrap();
        let o = pod(&p, Some((12.0, 50.0 * 1024.0 * 1024.0)));
        assert_eq!(o.cols, ["1/2", "CrashLoopBackOff", "5", "12m", "50Mi"]);
        assert_eq!(o.health, Health::Bad);
        let svc: Value = serde_json::from_str(r#"{"metadata":{"name":"web","namespace":"shop"},"spec":{"type":"NodePort","clusterIP":"10.43.0.9","ports":[{"port":80,"nodePort":30080,"protocol":"TCP"}]}}"#).unwrap();
        assert_eq!(other("Service", &svc).unwrap().cols, ["NodePort", "10.43.0.9", "80:30080/TCP"]);
        let dep: Value = serde_json::from_str(r#"{"metadata":{"name":"web","namespace":"shop"},"spec":{"replicas":3},"status":{"readyReplicas":2}}"#).unwrap();
        let d = other("Deployment", &dep).unwrap();
        assert_eq!((d.cols[1].as_str(), d.health), ("2/3", Health::Busy));
        let ca: Value = serde_json::from_str(r#"{"metadata":{"name":"kube-root-ca.crt","namespace":"shop"}}"#).unwrap();
        assert!(other("ConfigMap", &ca).is_none());
    }

    #[test]
    fn the_context_rename_only_touches_names() {
        let s = up_script("/Users/someone");
        assert!(s.contains(r"s/^( *(- )?(name|cluster|user|current-context): )default$/\1patina/"));
        assert!(s.contains("cfg='/Users/someone/.kube/config'"));
        assert!(s.contains("--docker --disable=traefik --disable=servicelb"));
    }
}
