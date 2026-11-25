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

/// Running a VM needs Apple's virtualization entitlement. Keep a copy of this
/// executable in `~/.patina/bin`, signed (ad hoc) with that entitlement, and
/// refresh it whenever the app changes.
fn ensure_helper() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dst = paths::helper();
    let meta = fs::metadata(&exe).map_err(|e| e.to_string())?;
    let stamp = format!("{}:{:?}", meta.len(), meta.modified().ok());
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
