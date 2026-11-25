//! Where Patina keeps its engine: everything lives under `~/.patina`.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn root() -> PathBuf {
    home().join(".patina")
}
pub fn run() -> PathBuf {
    root().join("run")
}
pub fn vm() -> PathBuf {
    root().join("vm")
}
pub fn cache() -> PathBuf {
    root().join("cache")
}
pub fn bin() -> PathBuf {
    root().join("bin")
}
pub fn logs() -> PathBuf {
    root().join("logs")
}

pub fn docker_sock() -> PathBuf {
    run().join("docker.sock")
}
pub fn control_sock() -> PathBuf {
    run().join("engine.sock")
}
pub fn pid_file() -> PathBuf {
    run().join("engine.pid")
}
pub fn disk() -> PathBuf {
    vm().join("disk.raw")
}
pub fn efi_vars() -> PathBuf {
    vm().join("efi.vars")
}
pub fn seed() -> PathBuf {
    vm().join("seed.iso")
}
pub fn mac_file() -> PathBuf {
    vm().join("mac")
}
/// Written by the guest (through the shared home folder) once first-boot provisioning is done.
pub fn provisioned() -> PathBuf {
    vm().join("provisioned")
}
pub fn console_log() -> PathBuf {
    logs().join("console.log")
}
pub fn engine_log() -> PathBuf {
    logs().join("engine.log")
}
pub fn config() -> PathBuf {
    root().join("config.json")
}
/// A signed copy of this executable that runs the VM.
pub fn helper() -> PathBuf {
    bin().join("patina-engine")
}
pub fn docker_cli() -> PathBuf {
    bin().join("docker")
}
pub fn compose_cli() -> PathBuf {
    bin().join("docker-compose")
}
pub fn buildx_cli() -> PathBuf {
    bin().join("docker-buildx")
}

pub fn ensure() {
    for d in [run(), vm(), cache(), bin(), logs()] {
        let _ = std::fs::create_dir_all(d);
    }
}
