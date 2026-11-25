//! Engine settings, stored in `~/.patina/config.json`.

use serde::{Deserialize, Serialize};

use super::paths;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    pub cpus: u32,
    pub memory_gb: u32,
    pub disk_gb: u32,
    /// Run linux/amd64 images with Rosetta.
    pub rosetta: bool,
    /// Start the engine when Patina opens.
    pub autostart: bool,
    /// Open Patina when you sign in.
    pub login: bool,
    /// Make `patina` the current docker context when the engine starts.
    pub set_context: bool,
    /// Put `~/.patina/bin` first on PATH through the shell profile.
    pub shell_path: bool,
    /// Keep the status item in the menu bar; closing the window keeps Patina there.
    pub menubar: bool,
    /// Run the k3s cluster (and forward its API to localhost:6443).
    pub kubernetes: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        let host = host_cpus();
        EngineConfig { cpus: host.clamp(1, 4), memory_gb: 4, disk_gb: 64, rosetta: true, autostart: true, login: false, set_context: true, shell_path: true, menubar: true, kubernetes: false }
    }
}

pub fn host_cpus() -> u32 {
    std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(2)
}

pub fn host_mem_gb() -> u32 {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.memsize"]).output() {
            if let Ok(v) = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>() {
                return (v >> 30) as u32;
            }
        }
    }
    8
}

pub fn load() -> EngineConfig {
    std::fs::read_to_string(paths::config()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(cfg: &EngineConfig) -> Result<(), String> {
    paths::ensure();
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(paths::config(), text).map_err(|e| e.to_string())?;
    set_login_item(cfg.login);
    Ok(())
}

/// Open Patina at login with a LaunchAgent (macOS).
fn set_login_item(on: bool) {
    #[cfg(target_os = "macos")]
    {
        let dir = paths::home().join("Library/LaunchAgents");
        let plist = dir.join("local.patina.app.plist");
        if !on {
            let _ = std::fs::remove_file(&plist);
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let exe = stable_exe(&exe);
        let _ = std::fs::create_dir_all(&dir);
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>local.patina.app</string>
  <key>ProgramArguments</key><array><string>{}</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#,
            xml_escape(&exe.display().to_string())
        );
        let _ = std::fs::write(plist, body);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = on;
}

/// Homebrew installs into a versioned Cellar folder that an upgrade removes.
/// Point at its `opt` link instead, which always leads to the current version:
/// `/opt/homebrew/Cellar/patina/0.1.0/bin/patina` becomes `/opt/homebrew/opt/patina/bin/patina`.
#[cfg(any(target_os = "macos", test))]
fn stable_exe(path: &std::path::Path) -> std::path::PathBuf {
    let parts: Vec<_> = path.components().collect();
    match parts.iter().position(|c| c.as_os_str() == "Cellar") {
        Some(i) if parts.len() > i + 3 => {
            let mut out: std::path::PathBuf = parts[..i].iter().collect();
            out.push("opt");
            out.push(parts[i + 1]);
            out.extend(&parts[i + 3..]);
            out
        }
        _ => path.to_path_buf(),
    }
}

#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::stable_exe;

    #[test]
    fn homebrew_paths_survive_upgrades() {
        let app = Path::new("/opt/homebrew/Cellar/patina/0.1.0/Patina.app/Contents/MacOS/Patina");
        assert_eq!(stable_exe(app), Path::new("/opt/homebrew/opt/patina/Patina.app/Contents/MacOS/Patina"));
        let bin = Path::new("/usr/local/Cellar/patina/0.2.0_1/bin/patina");
        assert_eq!(stable_exe(bin), Path::new("/usr/local/opt/patina/bin/patina"));
        let dev = Path::new("/Users/me/patina/target/release/patina");
        assert_eq!(stable_exe(dev), dev);
    }
}
