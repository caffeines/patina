//! Open a command in your terminal app: Terminal, Warp, iTerm or Ghostty.
//!
//! The command goes in a small script in `~/.patina/run` that deletes itself
//! when it starts, and the app opens it with `open -b`. Each of these
//! terminals runs a shell script it's asked to open, so they all work the
//! same way. The app is a setting (`terminal` in `~/.patina/config.json`).

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

/// The setting, or Terminal when it's unset or names an unknown app.
pub fn chosen(setting: &str) -> &'static str {
    APPS.iter().map(|a| a.0).find(|id| *id == setting).unwrap_or(DEFAULT)
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
    let app = chosen(setting);
    let name = APPS.iter().find(|a| a.0 == app).map_or("Terminal", |a| a.1);
    let path = write_script(&paths::run(), words)?;
    let ok = std::process::Command::new("/usr/bin/open").arg("-b").arg(app).arg(&path).status().is_ok_and(|s| s.success());
    if ok {
        Ok(())
    } else {
        let _ = std::fs::remove_file(&path);
        Err(format!("Couldn't open {name}."))
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
    /// command: `PATINA_TERMINAL=dev.warp.Warp-Stable cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn the_chosen_app_runs_the_command() {
        let app = std::env::var("PATINA_TERMINAL").unwrap_or_default();
        let mark = std::env::temp_dir().join(format!("patina-term-ran-{}", std::process::id()));
        let cmd = format!("echo \"Patina opened this in $TERM_PROGRAM. You can close it.\"; echo \"$TERM_PROGRAM\" > '{}'", mark.display());
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

    #[test]
    fn falls_back_to_terminal() {
        assert_eq!(chosen("dev.warp.Warp-Stable"), "dev.warp.Warp-Stable");
        assert_eq!(chosen(""), DEFAULT);
        assert_eq!(chosen("com.example.unknown"), DEFAULT);
        assert_eq!(installed().first().map(|a| a.0), Some(DEFAULT));
    }
}
