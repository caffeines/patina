//! First-run setup: the `docker` and `docker-compose` commands, and the VM disk.
//!
//! The disk is Debian's official generic cloud image, verified against the
//! SHA-512 list published next to it, grown to the configured size as a sparse
//! file. A small ISO labelled `cidata` carries the first-boot provisioning.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha512};

use super::{config, guest, paths};

const IMAGE_BASE: &str = "https://cloud.debian.org/images/cloud/trixie/latest/";

fn image_name() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "debian-13-generic-arm64.tar.xz"
    } else {
        "debian-13-generic-amd64.tar.xz"
    }
}

fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

/// The engine has a disk and its first-boot seed.
pub fn installed() -> bool {
    paths::disk().exists() && paths::seed().exists()
}

/// Progress callback: fraction done (negative when unknown) and a short note.
pub type Report<'a> = &'a dyn Fn(f64, &str);

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("patina/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn mb(b: u64) -> String {
    format!("{:.0} MB", b as f64 / 1e6)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn get_text(url: &str) -> Result<String, String> {
    agent().get(url).call().map_err(|e| format!("Couldn't reach {url}: {e}"))?.into_string().map_err(|e| e.to_string())
}

/// Download `url` to `dest` with progress, returning the SHA-512 of the bytes.
fn download(url: &str, dest: &Path, report: Report, label: &str) -> Result<String, String> {
    let resp = agent().get(url).call().map_err(|e| format!("Couldn't download {label}: {e}"))?;
    let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok());
    let mut reader = resp.into_reader();
    let part = dest.with_extension("part");
    let mut file = File::create(&part).map_err(|e| e.to_string())?;
    let mut hasher = Sha512::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    let mut last = Instant::now() - Duration::from_secs(1);
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("The download of {label} was interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if last.elapsed() > Duration::from_millis(150) {
            last = Instant::now();
            match total {
                Some(t) => report(done as f64 / t as f64, &format!("{} of {}", mb(done), mb(t))),
                None => report(-1.0, &mb(done)),
            }
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&part, dest).map_err(|e| e.to_string())?;
    Ok(hex(&hasher.finalize()))
}

fn sha512_file(path: &Path) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha512::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string())
    }
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
}

/// "docker-28.5.2.tgz" style names in Docker's static download index.
fn latest_docker(index: &str) -> Option<String> {
    let mut best: Option<(Vec<u32>, String)> = None;
    for part in index.split("docker-").skip(1) {
        let Some(end) = part.find(".tgz") else { continue };
        let v = &part[..end];
        if !v.chars().all(|c| c.is_ascii_digit() || c == '.') || v.is_empty() {
            continue;
        }
        let key: Vec<u32> = v.split('.').filter_map(|n| n.parse().ok()).collect();
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, v.to_string()));
        }
    }
    best.map(|(_, v)| v)
}

/// Install `docker` and `docker-compose` into `~/.patina/bin`, and add a
/// `patina` docker context (without switching to it).
pub fn install_cli(report: Report) -> Result<(), String> {
    paths::ensure();
    if !paths::docker_cli().exists() {
        report(-1.0, "Finding the latest docker command");
        let index = get_text(&format!("https://download.docker.com/mac/static/stable/{}/", arch()))?;
        let version = latest_docker(&index).ok_or("Couldn't find a docker release to download.")?;
        let tgz = paths::cache().join(format!("docker-{version}.tgz"));
        download(&format!("https://download.docker.com/mac/static/stable/{}/docker-{version}.tgz", arch()), &tgz, report, "the docker command")?;
        let tmp = paths::cache().join("docker-cli");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        run(Command::new("/usr/bin/tar").arg("-xzf").arg(&tgz).arg("-C").arg(&tmp).arg("docker/docker"))?;
        fs::rename(tmp.join("docker/docker"), paths::docker_cli()).map_err(|e| e.to_string())?;
        make_executable(&paths::docker_cli());
        let _ = fs::remove_dir_all(&tmp);
        let _ = fs::remove_file(&tgz);
    }
    if !paths::compose_cli().exists() {
        let url = format!("https://github.com/docker/compose/releases/latest/download/docker-compose-darwin-{}", arch());
        download(&url, &paths::compose_cli(), report, "docker compose")?;
        make_executable(&paths::compose_cli());
    }
    report(1.0, "done");
    let docker = paths::docker_cli();
    let has_context = Command::new(&docker).args(["context", "inspect", "patina"]).output().is_ok_and(|o| o.status.success());
    if !has_context {
        let host = format!("host=unix://{}", paths::docker_sock().display());
        let _ = Command::new(&docker).args(["context", "create", "patina", "--description", "Patina", "--docker", &host]).output();
    }
    Ok(())
}

fn random_mac() -> String {
    let mut b = [0u8; 3];
    if let Ok(mut f) = File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    }
    format!("52:54:00:{:02x}:{:02x}:{:02x}", b[0], b[1], b[2])
}

/// Make the VM disk and its first-boot seed.
pub fn create_disk(report: Report) -> Result<(), String> {
    paths::ensure();
    let cfg = config::load();
    if !paths::config().exists() {
        config::save(&cfg)?;
    }
    if !paths::disk().exists() {
        let name = image_name();
        let archive = paths::cache().join(name);
        report(-1.0, "Checking the Debian image");
        let sums = get_text(&format!("{IMAGE_BASE}SHA512SUMS"))?;
        let want = sums
            .lines()
            .find(|l| l.trim_end().ends_with(name))
            .and_then(|l| l.split_whitespace().next())
            .ok_or("Debian's checksum list doesn't include the image.")?
            .to_string();
        let have = if archive.exists() {
            report(-1.0, "Verifying the image");
            sha512_file(&archive)?
        } else {
            String::new()
        };
        if have != want {
            let got = download(&format!("{IMAGE_BASE}{name}"), &archive, report, "the Debian image")?;
            if got != want {
                let _ = fs::remove_file(&archive);
                return Err("The downloaded image didn't match Debian's checksum. Try again.".into());
            }
        }
        report(-1.0, "Unpacking the disk image");
        let tmp = paths::vm().join("unpack");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        run(Command::new("/usr/bin/tar").arg("-xJf").arg(&archive).arg("-C").arg(&tmp).arg("disk.raw"))?;
        let raw = tmp.join("disk.raw");
        let f = fs::OpenOptions::new().write(true).open(&raw).map_err(|e| e.to_string())?;
        let size = (cfg.disk_gb.max(16) as u64) << 30;
        if f.metadata().map(|m| m.len()).unwrap_or(0) < size {
            f.set_len(size).map_err(|e| e.to_string())?;
        }
        drop(f);
        report(-1.0, "Preparing the boot loader");
        let kernel = kernel_version(name);
        patch_boot(&raw, kernel.as_deref())?;
        fs::rename(&raw, paths::disk()).map_err(|e| e.to_string())?;
        let _ = fs::remove_dir_all(&tmp);
    }
    if !paths::mac_file().exists() {
        fs::write(paths::mac_file(), random_mac()).map_err(|e| e.to_string())?;
    }
    if !paths::seed().exists() {
        report(-1.0, "Writing the first-boot setup");
        write_seed()?;
    }
    report(1.0, "done");
    Ok(())
}

/// Boot the kernel with its console on the VM's virtio console (`hvc0`), so
/// every boot, including the first, is logged to `~/.patina/logs/console.log`.
///
/// The image's EFI partition holds a three-line `grub.cfg` that hands off to
/// Debian's config. This replaces it with one that boots `/vmlinuz` directly
/// with our kernel arguments, and falls back to Debian's config otherwise.
/// The image's kernel version, from the package manifest Debian publishes next to it.
fn kernel_version(image: &str) -> Option<String> {
    let manifest = get_text(&format!("{IMAGE_BASE}{}.json", image.trim_end_matches(".tar.xz"))).ok()?;
    let v: serde_json::Value = serde_json::from_str(&manifest).ok()?;
    let items = v["items"].as_array()?;
    items.iter().flat_map(|i| i["data"]["packages"].as_array().into_iter().flatten()).find_map(|p| {
        let name = p["name"].as_str()?;
        let ver = name.strip_prefix("linux-image-")?;
        ver.starts_with(|c: char| c.is_ascii_digit()).then(|| ver.to_string())
    })
}

fn patch_boot(raw: &Path, kernel: Option<&str>) -> Result<(), String> {
    let out = Command::new("/usr/bin/hdiutil")
        .args(["attach", "-nomount", "-imagekey", "diskimage-class=CRawDiskImage"])
        .arg(raw)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("Couldn't open the disk image: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let whole = text.lines().next().and_then(|l| l.split_whitespace().next()).unwrap_or("").to_string();
    let esp = text.lines().find(|l| l.contains("EFI")).and_then(|l| l.split_whitespace().next()).map(str::to_string);
    let result = (|| -> Result<(), String> {
        let esp = esp.ok_or("The disk image has no EFI partition.")?;
        let mnt = paths::vm().join("esp");
        let _ = fs::create_dir_all(&mnt);
        run(Command::new("/usr/sbin/diskutil").args(["mount", "-mountPoint"]).arg(&mnt).arg(&esp))?;
        let cfg = mnt.join("EFI/debian/grub.cfg");
        let r = (|| -> Result<(), String> {
            let old = fs::read_to_string(&cfg).map_err(|e| format!("Couldn't read the boot config: {e}"))?;
            let uuid = old
                .split_whitespace()
                .skip_while(|w| *w != "search.fs_uuid")
                .nth(1)
                .ok_or("Unexpected boot config.")?
                .to_string();
            let (vmlinuz, initrd) = match kernel {
                Some(k) => (format!("/boot/vmlinuz-{k}"), format!("/boot/initrd.img-{k}")),
                None => ("/vmlinuz".to_string(), "/initrd.img".to_string()),
            };
            let new = format!(
                "search.fs_uuid {uuid} root\nset prefix=($root)'/boot/grub'\nif [ -e ($root){vmlinuz} ]; then\n  linux ($root){vmlinuz} root=UUID={uuid} ro console=hvc0\n  initrd ($root){initrd}\n  boot\nfi\nconfigfile $prefix/grub.cfg\n"
            );
            fs::write(&cfg, new).map_err(|e| e.to_string())
        })();
        let _ = Command::new("/usr/sbin/diskutil").arg("unmount").arg(&mnt).output();
        let _ = fs::remove_dir(&mnt);
        r
    })();
    if !whole.is_empty() {
        let _ = Command::new("/usr/bin/hdiutil").args(["detach", &whole]).output();
    }
    result
}

pub fn write_seed() -> Result<(), String> {
    let dir = paths::vm().join("seed-src");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let home = paths::home().display().to_string();
    fs::write(dir.join("user-data"), guest::user_data(&home)).map_err(|e| e.to_string())?;
    fs::write(dir.join("meta-data"), guest::meta_data()).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(paths::seed());
    run(Command::new("/usr/bin/hdiutil")
        .args(["makehybrid", "-iso", "-joliet", "-default-volume-name", "cidata", "-o"])
        .arg(paths::seed())
        .arg(&dir))?;
    let _ = fs::remove_dir_all(&dir);
    Ok(())
}

/// Remove the engine's disk so the next setup starts fresh (keeps the downloads).
pub fn reset() -> Result<(), String> {
    for p in [paths::disk(), paths::seed(), paths::efi_vars(), paths::provisioned()] {
        if p.exists() {
            fs::remove_file(&p).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_newest_docker() {
        let index = r#"<a href="docker-27.5.1.tgz">docker-27.5.1.tgz</a> <a href="docker-28.10.0.tgz">x</a> <a href="docker-28.9.3.tgz">x</a> <a href="docker-rootless-extras-28.0.0.tgz">x</a>"#;
        assert_eq!(latest_docker(index).as_deref(), Some("28.10.0"));
    }

    #[test]
    fn macs_are_locally_administered() {
        let m = random_mac();
        assert!(m.starts_with("52:54:00:"));
        assert_eq!(m.len(), 17);
    }
}
