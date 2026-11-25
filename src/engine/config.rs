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
}

impl Default for EngineConfig {
    fn default() -> Self {
        let host = host_cpus();
        EngineConfig { cpus: host.clamp(1, 4), memory_gb: 4, disk_gb: 64, rosetta: true, autostart: true, login: false }
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

#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
