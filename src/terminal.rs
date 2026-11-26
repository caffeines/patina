//! Open a command in your terminal app: Terminal, Warp, iTerm or Ghostty on
//! macOS; on Linux, the terminal emulators found on PATH.
//!
//! The command goes in a small script in `~/.patina/run` that deletes itself
//! when it starts. On macOS the app opens it with `open -b`: each of these
//! terminals runs a shell script it's asked to open, so they all work the
//! same way. On Linux each terminal is given the script as the command to
//! run, with its own flag for that. The app is a setting (`terminal` in
//! `~/.patina/config.json`).

use std::path::{Path, PathBuf};

use crate::engine::paths;

pub use platform::{chosen, installed, name};

#[cfg(target_os = "macos")]
mod platform {
    use std::path::{Path, PathBuf};

    use crate::engine::paths;

    /// (bundle id, name, app bundle) of the terminals Patina knows.
    pub const APPS: [(&str, &str, &str); 4] = [
        ("com.apple.Terminal", "Terminal", "Terminal.app"),
        ("dev.warp.Warp-Stable", "Warp", "Warp.app"),
        ("com.googlecode.iterm2", "iTerm", "iTerm.app"),
        ("com.mitchellh.ghostty", "Ghostty", "Ghostty.app"),
    ];

    pub const DEFAULT: &str = "com.apple.Terminal";

    fn app_dirs() -> Vec<PathBuf> {
        vec![PathBuf::from("/Applications"), PathBuf::from("/System/Applications/Utilities"), paths::home().join("Applications")]
    }

    /// The known terminals on this Mac: Terminal always, then the others found.
    /// Looked up once.
    pub fn installed() -> &'static [(&'static str, &'static str)] {
        static FOUND: std::sync::OnceLock<Vec<(&'static str, &'static str)>> = std::sync::OnceLock::new();
        FOUND.get_or_init(|| {
            let dirs = app_dirs();
            APPS.iter()
                .filter(|(id, _, app)| *id == DEFAULT || dirs.iter().any(|d| d.join(app).exists()))
                .map(|(id, name, _)| (*id, *name))
                .collect()
        })
    }

    /// The terminal `id` names, for messages.
    pub fn name(id: &str) -> &'static str {
        APPS.iter().find(|a| a.0 == id).map_or("Terminal", |a| a.1)
    }

    /// The setting, or Terminal when it's unset or names an unknown app.
    pub fn chosen(setting: &str) -> &'static str {
        APPS.iter().map(|a| a.0).find(|id| *id == setting).unwrap_or(DEFAULT)
    }

    /// Open `script` in the terminal app `id`.
    pub fn launch(id: &str, script: &Path) -> bool {
        std::process::Command::new("/usr/bin/open").arg("-b").arg(id).arg(script).status().is_ok_and(|s| s.success())
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    /// (command, name, the arguments that come before the command to run) of
    /// the terminals Patina knows, most preferred first. The first two run
    /// the desktop's default terminal.
    pub const APPS: [(&str, &str, &[&str]); 12] = [
        ("xdg-terminal-exec", "Default", &[]),
        ("x-terminal-emulator", "Default", &["-e"]),
        ("gnome-terminal", "GNOME Terminal", &["--"]),
        ("ptyxis", "Ptyxis", &["--"]),
        ("konsole", "Konsole", &["-e"]),
        ("xfce4-terminal", "Xfce Terminal", &["-x"]),
        ("kitty", "kitty", &[]),
        ("alacritty", "Alacritty", &["-e"]),
        ("wezterm", "WezTerm", &["start", "--"]),
        ("ghostty", "Ghostty", &["-e"]),
        ("foot", "foot", &[]),
        ("xterm", "XTerm", &["-e"]),
    ];

    /// The most the Settings switcher has room for.
    const SHOWN: usize = 4;

    /// The known terminals on PATH, most preferred first, with at most one
    /// default. Looked up once.
    pub fn installed() -> &'static [(&'static str, &'static str)] {
        static FOUND: std::sync::OnceLock<Vec<(&'static str, &'static str)>> = std::sync::OnceLock::new();
        FOUND.get_or_init(|| {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
            found(|cmd| dirs.iter().any(|d| executable(&d.join(cmd))))
        })
    }

    pub(super) fn found(on_path: impl Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
        let mut out: Vec<(&str, &str)> = Vec::new();
        for (cmd, name, _) in APPS {
            let another_default = name == "Default" && out.iter().any(|a| a.1 == "Default");
            if out.len() < SHOWN && !another_default && on_path(cmd) {
                out.push((cmd, name));
            }
        }
        out
    }

    fn executable(p: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }

    /// The terminal `id` names, for messages.
    pub fn name(id: &str) -> &'static str {
        APPS.iter().find(|a| a.0 == id).map_or("a terminal", |a| a.1)
    }

    /// The setting when it names a terminal that's here, otherwise the
    /// first one found.
    pub fn chosen(setting: &str) -> &'static str {
        let here = installed();
        here.iter().map(|a| a.0).find(|id| *id == setting).or_else(|| here.first().map(|a| a.0)).unwrap_or(APPS[0].0)
    }

    /// The command line that runs `script` in the terminal `id`.
    pub(super) fn command_line(id: &str, script: &Path) -> Option<Vec<String>> {
        let (cmd, _, before) = APPS.iter().find(|a| a.0 == id)?;
        let mut words = vec![cmd.to_string()];
        words.extend(before.iter().map(|w| w.to_string()));
        words.push(script.display().to_string());
        Some(words)
    }

    /// Run `script` in the terminal `id`. The terminal keeps running on its
    /// own; a thread waits for it so it doesn't linger as a zombie.
    pub fn launch(id: &str, script: &Path) -> bool {
        let Some(words) = command_line(id, script) else { return false };
        let child = Command::new(&words[0]).args(&words[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        match child {
            Ok(mut c) => {
                std::thread::spawn(move || c.wait());
                true
            }
            Err(_) => false,
        }
    }
}

/// One shell word: plain when it's safe, otherwise single-quoted.
pub fn quote(a: &str) -> String {
    if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/' | ':' | '=' | '@' | '+' | ',')) {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

/// The script that runs `words` in the terminal.
fn script(words: &[String]) -> String {
    let line: Vec<String> = words.iter().map(|w| quote(w)).collect();
    format!("#!/bin/sh\nrm -f \"$0\"\nclear\nexec {}\n", line.join(" "))
}

fn write_script(dir: &Path, words: &[String]) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path = dir.join(format!("patina-shell-{stamp}.command"));
    std::fs::write(&path, script(words)).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Run `words` in a new window or tab of the terminal app `setting` names.
pub fn open(setting: &str, words: &[String]) -> Result<(), String> {
    if installed().is_empty() {
        return Err("No terminal app found. Install one, such as GNOME Terminal or Konsole.".into());
    }
    let app = chosen(setting);
    let path = write_script(&paths::run(), words)?;
    if platform::launch(app, &path) {
        Ok(())
    } else {
        let _ = std::fs::remove_file(&path);
        Err(format!("Couldn't open {}.", name(app)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_only_what_needs_it() {
        assert_eq!(quote("exec"), "exec");
        assert_eq!(quote("unix:///Users/me/.patina/run/docker.sock"), "unix:///Users/me/.patina/run/docker.sock");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn the_script_removes_itself_then_runs_the_command() {
        let words = ["/x/docker", "exec", "-it", "web", "sh", "-c", "exec bash"].map(String::from);
        assert_eq!(script(&words), "#!/bin/sh\nrm -f \"$0\"\nclear\nexec /x/docker exec -it web sh -c 'exec bash'\n");
    }

    #[test]
    fn writes_a_private_executable_script() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("patina-term-{}", std::process::id()));
        let p = write_script(&dir, &["true".to_string()]).unwrap();
        assert!(p.extension().is_some_and(|e| e == "command"));
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Opens the terminal app in `PATINA_TERMINAL` and checks it ran the
    /// command: `PATINA_TERMINAL=dev.warp.Warp-Stable cargo test -- --ignored`
    /// (on Linux, a command such as `xterm`).
    #[test]
    #[ignore]
    fn the_chosen_app_runs_the_command() {
        let app = std::env::var("PATINA_TERMINAL").unwrap_or_default();
        let mark = std::env::temp_dir().join(format!("patina-term-ran-{}", std::process::id()));
        let cmd = format!("echo \"Patina opened this in $TERM_PROGRAM. You can close it.\"; echo \"${{TERM_PROGRAM:-$TERM}}\" > '{}'", mark.display());
        open(&app, &["/bin/sh".to_string(), "-c".to_string(), cmd]).unwrap();
        let ran = (0..40).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(250));
            mark.exists()
        });
        let got = std::fs::read_to_string(&mark).unwrap_or_default();
        let _ = std::fs::remove_file(&mark);
        assert!(ran, "{} didn't run the command", chosen(&app));
        eprintln!("ran in: {}", got.trim());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn falls_back_to_terminal() {
        use platform::DEFAULT;
        assert_eq!(chosen("dev.warp.Warp-Stable"), "dev.warp.Warp-Stable");
        assert_eq!(chosen(""), DEFAULT);
        assert_eq!(chosen("com.example.unknown"), DEFAULT);
        assert_eq!(installed().first().map(|a| a.0), Some(DEFAULT));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn finds_terminals_on_path() {
        let have = |list: &'static [&'static str]| move |cmd: &str| list.contains(&cmd);
        let ids = |v: Vec<(&'static str, &'static str)>| v.into_iter().map(|a| a.0).collect::<Vec<_>>();
        // One default, preferred order, and no more than the switcher shows.
        let all = platform::found(have(&["xterm", "konsole", "x-terminal-emulator", "xdg-terminal-exec", "kitty", "foot"]));
        assert_eq!(ids(all), ["xdg-terminal-exec", "konsole", "kitty", "foot"]);
        assert_eq!(ids(platform::found(have(&["xterm"]))), ["xterm"]);
        assert!(platform::found(have(&[])).is_empty());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn runs_the_script_with_each_terminals_flag() {
        let s = Path::new("/home/me/.patina/run/patina-shell-1.command");
        let line = |id| platform::command_line(id, s).unwrap().join(" ");
        assert_eq!(line("gnome-terminal"), "gnome-terminal -- /home/me/.patina/run/patina-shell-1.command");
        assert_eq!(line("konsole"), "konsole -e /home/me/.patina/run/patina-shell-1.command");
        assert_eq!(line("wezterm"), "wezterm start -- /home/me/.patina/run/patina-shell-1.command");
        assert_eq!(line("kitty"), "kitty /home/me/.patina/run/patina-shell-1.command");
        assert!(platform::command_line("notepad", s).is_none());
    }
}
