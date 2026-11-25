//! Make the `docker` command use Patina, like Docker Desktop and OrbStack do:
//!
//! - **Docker context.** A `patina` context points at Patina's socket. When
//!   the engine finishes starting, Patina makes it the current context
//!   (`docker context use patina`). Turning the setting off switches back to
//!   `default`, if Patina's context is the current one.
//! - **PATH.** `~/.patina/shell/init.{zsh,bash,fish}` put `~/.patina/bin`
//!   first on PATH, and one marked line in the shell profile sources it. The
//!   line is removed again when the setting is turned off.
//! - **Plugins.** When the docker command can't find `compose` and `buildx`
//!   as plugins on its own, `~/.patina/bin` is added to
//!   `cliPluginsExtraDirs` in `~/.docker/config.json`, the way Docker
//!   Desktop adds its app's plugin folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::paths;

const CONTEXT: &str = "patina";
const MARK: &str = "# Added by Patina: puts Patina's docker command first on PATH";

fn docker(args: &[&str]) -> Result<String, String> {
    let cli = paths::docker_cli();
    if !cli.exists() {
        return Err("Patina's docker command isn't installed yet.".into());
    }
    let out = Command::new(&cli).args(args).env_remove("DOCKER_CONTEXT").env_remove("DOCKER_HOST").output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Create the `patina` context, or point it at Patina's socket if it drifted.
pub fn ensure_context() -> Result<(), String> {
    let host = format!("unix://{}", paths::docker_sock().display());
    match docker(&["context", "inspect", CONTEXT, "--format", "{{.Endpoints.docker.Host}}"]) {
        Ok(current) if current == host => Ok(()),
        Ok(_) => docker(&["context", "update", CONTEXT, "--docker", &format!("host={host}")]).map(|_| ()),
        Err(_) => docker(&["context", "create", CONTEXT, "--description", "Patina", "--docker", &format!("host={host}")]).map(|_| ()),
    }
}

/// The docker CLI's current context.
pub fn current_context() -> Option<String> {
    docker(&["context", "show"]).ok()
}

/// Make `patina` the current docker context.
pub fn use_context() -> Result<(), String> {
    ensure_context()?;
    if current_context().as_deref() == Some(CONTEXT) {
        return Ok(());
    }
    docker(&["context", "use", CONTEXT]).map(|_| ())
}

/// Hand the docker command back to the default context, if Patina has it.
pub fn release_context() -> Result<(), String> {
    if current_context().as_deref() == Some(CONTEXT) {
        docker(&["context", "use", "default"]).map(|_| ())
    } else {
        Ok(())
    }
}

/// Make `docker compose` and `docker buildx` work with Patina's docker.
pub fn ensure_plugins() -> Result<(), String> {
    if ["compose", "buildx"].iter().all(|p| docker(&[p, "version"]).is_ok()) {
        return Ok(());
    }
    let dir = std::env::var_os("DOCKER_CONFIG").map(PathBuf::from).unwrap_or_else(|| paths::home().join(".docker"));
    add_plugin_dir(&dir.join("config.json"), &paths::bin())
}

/// Add `dir` to `cliPluginsExtraDirs` in a docker config file, keeping
/// everything else in it.
fn add_plugin_dir(config: &Path, dir: &Path) -> Result<(), String> {
    let mut cfg: serde_json::Value = match fs::read_to_string(config) {
        Ok(t) if !t.trim().is_empty() => serde_json::from_str(&t).map_err(|e| format!("{} isn't valid JSON: {e}", config.display()))?,
        Ok(_) => serde_json::json!({}),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e.to_string()),
    };
    let dir = dir.to_string_lossy().into_owned();
    let list = cfg
        .as_object_mut()
        .ok_or("The docker config isn't a JSON object.")?
        .entry("cliPluginsExtraDirs")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or("cliPluginsExtraDirs in the docker config isn't a list.")?;
    if list.iter().any(|d| d.as_str() == Some(dir.as_str())) {
        return Ok(());
    }
    list.push(dir.into());
    // Tab-indented, as the docker command writes it; private, as it may hold credentials.
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b"\t"));
    serde::Serialize::serialize(&cfg, &mut ser).map_err(|e| e.to_string())?;
    out.push(b'\n');
    if let Some(parent) = config.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = config.with_extension("json.patina");
    fs::write(&tmp, out).map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    fs::rename(&tmp, config).map_err(|e| e.to_string())
}

fn shell_dir() -> PathBuf {
    paths::root().join("shell")
}

const POSIX_INIT: &str = r#"# Patina: put Patina's docker and docker-compose first on PATH.
case "$PATH" in
  "$HOME/.patina/bin"|"$HOME/.patina/bin:"*) ;;
  *) export PATH="$HOME/.patina/bin:$PATH" ;;
esac
"#;

const FISH_INIT: &str = r#"# Patina: put Patina's docker and docker-compose first on PATH.
if test "$PATH[1]" != "$HOME/.patina/bin"
    set -gx PATH $HOME/.patina/bin $PATH
end
"#;

/// Shell profiles to edit: zsh always (the macOS default), bash only when the user has a profile.
fn profiles() -> Vec<(PathBuf, &'static str)> {
    let home = paths::home();
    let mut out = vec![(home.join(".zprofile"), "zsh")];
    if home.join(".bash_profile").exists() {
        out.push((home.join(".bash_profile"), "bash"));
    } else if home.join(".profile").exists() {
        out.push((home.join(".profile"), "bash"));
    }
    out
}

fn fish_conf() -> Option<PathBuf> {
    let dir = paths::home().join(".config/fish");
    dir.exists().then(|| dir.join("conf.d/patina.fish"))
}

fn source_line(shell: &str) -> String {
    format!("source ~/.patina/shell/init.{shell} 2>/dev/null || :")
}

fn has_line(text: &str) -> bool {
    text.lines().any(|l| l.contains(".patina/shell/init."))
}

/// Add `profile`'s Patina line, keeping everything else as it was.
fn add_to_profile(profile: &Path, shell: &str) -> Result<bool, String> {
    let text = fs::read_to_string(profile).unwrap_or_default();
    if has_line(&text) {
        return Ok(false);
    }
    let mut new = text.clone();
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    if !new.is_empty() {
        new.push('\n');
    }
    new.push_str(MARK);
    new.push('\n');
    new.push_str(&source_line(shell));
    new.push('\n');
    fs::write(profile, new).map_err(|e| format!("Couldn't update {}: {e}", profile.display()))?;
    Ok(true)
}

fn remove_from_profile(profile: &Path) -> Result<(), String> {
    let Ok(text) = fs::read_to_string(profile) else { return Ok(()) };
    if !has_line(&text) {
        return Ok(());
    }
    let kept: Vec<&str> = text.lines().filter(|l| *l != MARK && !l.contains(".patina/shell/init.")).collect();
    let mut new = kept.join("\n");
    while new.ends_with("\n\n") || new.ends_with('\n') {
        new.pop();
    }
    if !new.is_empty() {
        new.push('\n');
    }
    fs::write(profile, new).map_err(|e| format!("Couldn't update {}: {e}", profile.display()))
}

/// Whether the zsh profile has Patina's line.
pub fn shell_installed() -> bool {
    fs::read_to_string(paths::home().join(".zprofile")).is_ok_and(|t| has_line(&t))
}

/// Put `~/.patina/bin` on PATH for new terminals. Returns the files changed.
pub fn install_shell() -> Result<Vec<PathBuf>, String> {
    let dir = shell_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join("init.zsh"), POSIX_INIT).map_err(|e| e.to_string())?;
    fs::write(dir.join("init.bash"), POSIX_INIT).map_err(|e| e.to_string())?;
    fs::write(dir.join("init.fish"), FISH_INIT).map_err(|e| e.to_string())?;
    let mut changed = Vec::new();
    for (profile, shell) in profiles() {
        if add_to_profile(&profile, shell)? {
            changed.push(profile);
        }
    }
    if let Some(conf) = fish_conf() {
        if !conf.exists() {
            if let Some(parent) = conf.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&conf, "source ~/.patina/shell/init.fish\n").map_err(|e| e.to_string())?;
            changed.push(conf);
        }
    }
    Ok(changed)
}

/// Take Patina's line back out of the shell profiles.
pub fn remove_shell() -> Result<(), String> {
    for (profile, _) in profiles() {
        remove_from_profile(&profile)?;
    }
    if let Some(conf) = fish_conf() {
        let _ = fs::remove_file(conf);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_plugin_dir_once_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("patina-test-{}", std::process::id()));
        let config = dir.join("config.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&config, "{\n\t\"auths\": {\"ghcr.io\": {}},\n\t\"currentContext\": \"patina\"\n}").unwrap();
        let bin = Path::new("/Users/someone/.patina/bin");
        add_plugin_dir(&config, bin).unwrap();
        add_plugin_dir(&config, bin).unwrap();
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v["cliPluginsExtraDirs"], serde_json::json!(["/Users/someone/.patina/bin"]));
        assert_eq!(v["currentContext"], "patina");
        assert!(v["auths"]["ghcr.io"].is_object());
        // A missing file gets created.
        let fresh = dir.join("new/config.json");
        add_plugin_dir(&fresh, bin).unwrap();
        assert!(fs::read_to_string(&fresh).unwrap().contains("cliPluginsExtraDirs"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_lines_are_added_once_and_removed_cleanly() {
        let dir = std::env::temp_dir().join(format!("patina-profile-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(".zprofile");
        let original = "# Added by OrbStack\nsource ~/.orbstack/shell/init.zsh 2>/dev/null || :\n";
        fs::write(&p, original).unwrap();
        assert!(add_to_profile(&p, "zsh").unwrap());
        assert!(!add_to_profile(&p, "zsh").unwrap());
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.starts_with(original));
        assert!(text.ends_with("source ~/.patina/shell/init.zsh 2>/dev/null || :\n"));
        remove_from_profile(&p).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn posix_init_puts_patina_first() {
        let out = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("HOME=/h; PATH=/usr/local/bin:/usr/bin; {POSIX_INIT} echo \"$PATH\"; {POSIX_INIT} echo \"$PATH\""))
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines, ["/h/.patina/bin:/usr/local/bin:/usr/bin", "/h/.patina/bin:/usr/local/bin:/usr/bin"]);
    }
}
