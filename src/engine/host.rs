//! The Docker Engine already on this computer, which Patina uses on Linux.
//!
//! Linux runs containers natively, so there's no VM to boot: Patina talks to
//! the Docker Engine the person installed, through its socket. This finds
//! that socket and, when it doesn't answer, says why in words someone can act
//! on: Docker isn't installed, they can't use it, or it isn't running.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where Docker listens without `DOCKER_HOST` or rootless Docker.
const SYSTEM: &str = "/var/run/docker.sock";

/// Docker's socket: the one `DOCKER_HOST` names, then rootless Docker's in
/// `$XDG_RUNTIME_DIR`, then the system one.
pub fn socket() -> PathBuf {
    let host = std::env::var("DOCKER_HOST").ok();
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    pick(host.as_deref(), runtime.as_deref(), Path::exists)
}

fn pick(docker_host: Option<&str>, runtime: Option<&Path>, exists: impl Fn(&Path) -> bool) -> PathBuf {
    if let Some(p) = docker_host.and_then(|h| h.strip_prefix("unix://")).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    match runtime.map(|d| d.join("docker.sock")) {
        Some(p) if exists(&p) => p,
        _ => PathBuf::from(SYSTEM),
    }
}

/// Why Docker can't be used, and which of the first-run checks found it:
/// 0 finding it, 1 permission to use it, 2 it answering.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub step: usize,
    pub message: String,
}

/// Check that Docker answers on [`socket`].
pub fn check() -> Result<(), Problem> {
    let sock = socket();
    let s = UnixStream::connect(&sock).map_err(|e| explain(&sock, e.kind(), &e.to_string()))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
    let mut line = String::new();
    let sent = (&s).write_all(b"GET /_ping HTTP/1.0\r\nHost: docker\r\n\r\n");
    if sent.is_ok() && BufReader::new(&s).read_line(&mut line).is_ok() && answers(&line) {
        return Ok(());
    }
    Err(Problem {
        step: 2,
        message: format!("Docker Engine at {} isn't answering. Restart it with `sudo systemctl restart docker`, then try again.", sock.display()),
    })
}

/// The status line of Docker's reply to `/_ping` says it's fine.
fn answers(status_line: &str) -> bool {
    let mut words = status_line.split_whitespace();
    words.next().is_some_and(|w| w.starts_with("HTTP/")) && words.next() == Some("200")
}

fn explain(sock: &Path, kind: ErrorKind, detail: &str) -> Problem {
    let at = sock.display();
    match kind {
        ErrorKind::NotFound => Problem {
            step: 0,
            message: format!(
                "There's no Docker Engine at {at}. Install it (https://docs.docker.com/engine/install/), or start it with `sudo systemctl start docker`, then try again."
            ),
        },
        ErrorKind::PermissionDenied => Problem {
            step: 1,
            message: format!(
                "You don't have permission to use Docker at {at}. Add yourself to the docker group with `sudo usermod -aG docker $USER`, then sign out and back in."
            ),
        },
        ErrorKind::ConnectionRefused => {
            Problem { step: 2, message: "Docker Engine isn't running. Start it with `sudo systemctl start docker`, then try again.".into() }
        }
        _ => Problem { step: 2, message: format!("Couldn't reach Docker at {at}: {detail}") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_socket() {
        let none = |_: &Path| false;
        let all = |_: &Path| true;
        let run = Path::new("/run/user/1000");
        assert_eq!(pick(Some("unix:///tmp/d.sock"), Some(run), all), PathBuf::from("/tmp/d.sock"));
        assert_eq!(pick(Some("tcp://10.0.0.2:2375"), Some(run), all), PathBuf::from("/run/user/1000/docker.sock"));
        assert_eq!(pick(None, Some(run), all), PathBuf::from("/run/user/1000/docker.sock"));
        assert_eq!(pick(None, Some(run), none), PathBuf::from(SYSTEM));
        assert_eq!(pick(Some("unix://"), None, all), PathBuf::from(SYSTEM));
    }

    #[test]
    fn reads_the_ping_reply() {
        assert!(answers("HTTP/1.0 200 OK\r\n"));
        assert!(answers("HTTP/1.1 200 OK\r\n"));
        assert!(!answers("HTTP/1.1 500 Internal Server Error\r\n"));
        assert!(!answers(""));
        assert!(!answers("200"));
    }

    #[test]
    fn says_what_to_do() {
        let s = Path::new(SYSTEM);
        assert_eq!(explain(s, ErrorKind::NotFound, "").step, 0);
        let denied = explain(s, ErrorKind::PermissionDenied, "");
        assert_eq!(denied.step, 1);
        assert!(denied.message.contains("docker group"));
        assert_eq!(explain(s, ErrorKind::ConnectionRefused, "").step, 2);
    }

    #[test]
    fn a_missing_socket_is_step_0() {
        let dir = std::env::temp_dir().join(format!("patina-host-{}", std::process::id()));
        let e = UnixStream::connect(dir.join("docker.sock")).unwrap_err();
        assert_eq!(explain(&dir, e.kind(), "").step, 0);
    }
}
