//! The app's side of the engine: start it, stop it, ask how it's doing.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{paths, setup};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// starting | booting | installing | running | stopping | error
    pub state: String,
    pub message: String,
    pub first_boot: bool,
    /// Seconds since the engine process started.
    pub uptime: f64,
    /// The VM's state as Virtualization reports it.
    #[serde(default)]
    pub vm: String,
    /// The engine's revision ([`ENGINE_REV`] of the build it runs). Engines
    /// from before revisions report 0.
    #[serde(default)]
    pub rev: u32,
}

/// What the engine process can do, bumped whenever that changes, so an
/// engine left running across an update is noticed and restarted.
/// 1: forwards the Kubernetes API, reports its revision.
/// 2: upgrades the guest so `docker run` and `docker exec` output isn't cut off.
pub const ENGINE_REV: u32 = 2;

/// The running engine is older than this app and lacks what changed since.
pub fn stale(s: &Status) -> bool {
    s.rev < ENGINE_REV
}

/// This build of Patina: its executable's size and modification time.
fn build_stamp() -> String {
    std::env::current_exe()
        .and_then(fs::metadata)
        .map(|m| format!("{}:{:?}", m.len(), m.modified().ok()))
        .unwrap_or_default()
}

fn request(cmd: &str) -> Option<String> {
    let mut s = UnixStream::connect(paths::control_sock()).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
    writeln!(s, "{cmd}").ok()?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).ok()?;
    Some(line.trim().to_string())
}

/// `None` when the engine isn't running.
pub fn status() -> Option<Status> {
    request("status").and_then(|l| serde_json::from_str(&l).ok())
}

/// Ask the engine to shut down. Returns false when it wasn't running.
pub fn stop() -> bool {
    request("stop").is_some()
}

/// The last few lines of the engine's log, for error messages.
pub fn log_tail(n: usize) -> String {
    let text = fs::read_to_string(paths::engine_log()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Launch the engine helper in the background. It keeps running after the
/// app quits, like a system service.
pub fn start() -> Result<(), String> {
    if status().is_some() {
        return Ok(());
    }
    if !setup::installed() {
        return Err("Patina's engine isn't set up yet.".into());
    }
    if !cfg!(target_os = "macos") {
        return Err("Patina's engine needs macOS.".into());
    }
    paths::ensure();
    let helper = ensure_helper()?;
    let log = OpenOptions::new().create(true).append(true).open(paths::engine_log()).map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&helper);
    cmd.args(["engine", "run"]).stdin(Stdio::null()).stdout(log).stderr(err);
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map_err(|e| format!("Couldn't launch the engine: {e}"))?;
    for _ in 0..80 {
        std::thread::sleep(Duration::from_millis(100));
        if status().is_some() {
            return Ok(());
        }
    }
    Err(match log_tail(3) {
        t if t.is_empty() => "The engine didn't start.".into(),
        t => t,
    })
}

const ENTITLEMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.virtualization</key>
  <true/>
</dict>
</plist>
"#;

/// Whether `path` is signed with Apple's virtualization entitlement.
fn has_vz_entitlement(path: &std::path::Path) -> bool {
    Command::new("/usr/bin/codesign")
        .args(["-d", "--entitlements", "-", "--xml"])
        .arg(path)
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("com.apple.security.virtualization"))
}

/// Running a VM needs Apple's virtualization entitlement. A packaged
/// Patina.app is signed with it, so it runs the engine itself. A development
/// build isn't: keep a copy of it in `~/.patina/bin`, signed (ad hoc) with
/// the entitlement, and refresh it whenever the build changes.
fn ensure_helper() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if has_vz_entitlement(&exe) {
        return Ok(exe);
    }
    let dst = paths::helper();
    let stamp = build_stamp();
    let stamp_file = paths::vm().join("helper.stamp");
    if dst.exists() && fs::read_to_string(&stamp_file).ok().as_deref() == Some(stamp.as_str()) {
        return Ok(dst);
    }
    let tmp = dst.with_extension("new");
    fs::copy(&exe, &tmp).map_err(|e| format!("Couldn't prepare the engine: {e}"))?;
    let ent = paths::run().join("vz.entitlements");
    fs::write(&ent, ENTITLEMENTS).map_err(|e| e.to_string())?;
    let out = Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-", "--entitlements"])
        .arg(&ent)
        .arg(&tmp)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("Couldn't sign the engine: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    fs::write(stamp_file, stamp).map_err(|e| e.to_string())?;
    Ok(dst)
}
