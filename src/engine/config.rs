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
    /// The bundle id of the terminal app that shells open in (empty: Terminal).
    pub terminal: String,
}

impl Default for EngineConfig {
    fn default() -> Self {
        let host = host_cpus();
        // This computer's own Docker Engine needs no docker context or PATH
        // line, and Linux has neither Rosetta nor Patina's menu bar item.
        let vm = !super::NATIVE;
        EngineConfig {
            cpus: host.clamp(1, 4),
            memory_gb: 4,
            disk_gb: 64,
            rosetta: vm,
            autostart: true,
            login: false,
            set_context: vm,
            shell_path: vm,
            menubar: vm,
            kubernetes: false,
            terminal: String::new(),
        }
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

/// Open Patina at login: a LaunchAgent on macOS, an XDG autostart entry
/// (`~/.config/autostart/patina.desktop`) elsewhere.
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
    {
        let config = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| paths::home().join(".config"));
        let file = config.join("autostart").join("patina.desktop");
        if !on {
            let _ = std::fs::remove_file(&file);
            return;
        }
        // An AppImage runs from a temporary mount; $APPIMAGE is the file itself.
        let exe = match std::env::var_os("APPIMAGE") {
            Some(a) => std::path::PathBuf::from(a),
            None => match std::env::current_exe() {
                Ok(e) => stable_exe(&e),
                Err(_) => return,
            },
        };
        let _ = std::fs::create_dir_all(config.join("autostart"));
        let _ = std::fs::write(file, autostart_entry(&exe));
    }
}

/// The autostart entry that opens `exe` at login.
#[cfg(any(not(target_os = "macos"), test))]
fn autostart_entry(exe: &std::path::Path) -> String {
    format!("[Desktop Entry]\nType=Application\nName=Patina\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n", desktop_exec(&exe.display().to_string()))
}

/// One argument of a desktop entry's `Exec`, quoted as the spec asks: in
/// double quotes, with `"`, `` ` ``, `$` and `\` escaped by a backslash, and
/// `%` doubled. String values are unescaped before the quoting is read, so
/// each of those backslashes is written as two.
#[cfg(any(not(target_os = "macos"), test))]
fn desktop_exec(arg: &str) -> String {
    let mut out = String::from('"');
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Homebrew installs into a versioned Cellar folder that an upgrade removes.
/// Point at its `opt` link instead, which always leads to the current version:
/// `/opt/homebrew/Cellar/patina/0.1.0/bin/patina` becomes `/opt/homebrew/opt/patina/bin/patina`.
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

    use super::{autostart_entry, desktop_exec, stable_exe};

    #[test]
    fn autostart_quotes_the_path() {
        assert_eq!(desktop_exec("/usr/bin/patina"), r#""/usr/bin/patina""#);
        assert_eq!(desktop_exec("/home/a b/50%/patina"), r#""/home/a b/50%%/patina""#);
        assert_eq!(desktop_exec(r#"/x/"$`\/p"#), r#""/x/\\"\\$\\`\\\\/p""#);
        let entry = autostart_entry(Path::new("/opt/Patina.AppImage"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("\nExec=\"/opt/Patina.AppImage\"\n"));
    }

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
