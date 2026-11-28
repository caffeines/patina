//! What the guest does on first boot (cloud-init NoCloud seed).
//!
//! The VM is a stock Debian cloud image. On first boot cloud-init installs
//! Docker Engine and socat, and sets up four small services:
//!
//! - `patina-boot` mounts the Mac's home folder at the same path (so bind
//!   mounts like `-v ~/code:/src` work) and registers Rosetta for amd64.
//! - `patina-docker` serves `/run/docker.sock` on vsock port 2375.
//! - `patina-ports` connects vsock port 2376 to a guest TCP port named in
//!   the first line, which is how published ports reach localhost on the Mac.
//! - `patina-memory` reports the guest's memory use on vsock port 2377, which
//!   the engine sizes the memory balloon from. The kernel boots with
//!   [`KERNEL_ARGS`], which the balloon needs.
//!
//! socat runs with a long `-t`: by default it closes a connection half a
//! second after one side finishes sending, which cut off `docker run` and
//! `docker exec` output once the docker command had sent all its input.
//! [`upgrade`] brings VMs set up before that, or before `patina-memory`, up
//! to date.

use super::{PORT_DOCKER, PORT_FORWARD, PORT_MEMORY};

/// How long socat keeps a connection open after one side finishes sending:
/// long enough for any command's output.
const HALF_CLOSE: &str = "-t 86400";

/// Single-quote a path for /bin/sh.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The guest kernel's command line. `init_on_alloc=0` stops Linux zeroing
/// pages as it allocates them: Debian turns that on, and it would make the
/// balloon write to every page it takes, which macOS then commits to the VM
/// for good ([`super::balloon`]).
pub const KERNEL_ARGS: &str = "console=hvc0 init_on_alloc=0";

/// `/usr/local/sbin/patina-memory`, which `patina-memory.service` runs for
/// each connection.
const MEMORY_SCRIPT: &str = r#"#!/bin/sh
# The guest's memory use, for the balloon.
grep -E '^(MemTotal|MemAvailable):' /proc/meminfo
grep '^balloon_' /proc/vmstat
echo "PageSize: $(getconf PAGESIZE)"
for f in /sys/bus/virtio/drivers/virtio_balloon/virtio*/features; do
  echo "BalloonFeatures: $(cat "$f")"
done
case " $(cat /proc/cmdline) " in
  *" init_on_alloc=0 "*) echo "InitOnAlloc: 0" ;;
  *) echo "InitOnAlloc: 1" ;;
esac
"#;

fn memory_unit() -> String {
    format!(
        "[Unit]
Description=Patina memory channel
[Service]
ExecStart=/usr/bin/socat VSOCK-LISTEN:{PORT_MEMORY},reuseaddr,fork EXEC:/usr/local/sbin/patina-memory
Restart=always
RestartSec=1
[Install]
WantedBy=multi-user.target
"
    )
}

/// `text` with each line indented by `n` spaces, for a YAML block.
fn indent(text: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    text.lines().map(|l| if l.is_empty() { "\n".to_string() } else { format!("{pad}{l}\n") }).collect()
}

pub fn meta_data() -> String {
    "instance-id: patina-engine-1\nlocal-hostname: patina\n".to_string()
}

pub fn user_data(home: &str) -> String {
    let h = sh_quote(home);
    let marker = sh_quote(&format!("{home}/.patina/vm/provisioned"));
    // binfmt_misc entry for x86_64 ELF binaries, as documented for Rosetta in Linux VMs.
    let rosetta = r":rosetta:M::\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x3e\x00:\xff\xff\xff\xff\xff\xfe\xfe\x00\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\xff:/mnt/rosetta/rosetta:OCF";
    let memory_script = indent(MEMORY_SCRIPT, 6);
    let memory_unit = indent(&memory_unit(), 6);
    format!(
        r#"#cloud-config
hostname: patina
manage_etc_hosts: true
ssh_pwauth: false
growpart:
  mode: auto
  devices: ["/"]
resize_rootfs: true
package_update: true
package_upgrade: false
packages:
  - docker.io
  - socat
write_files:
  - path: /usr/local/sbin/patina-boot
    permissions: "0755"
    content: |
      #!/bin/sh
      # Shares from the Mac: the home folder, and Rosetta when it is on.
      mkdir -p {h} /mnt/rosetta
      mountpoint -q {h} || mount -t virtiofs patina-home {h} || true
      if mountpoint -q /mnt/rosetta || mount -t virtiofs rosetta /mnt/rosetta 2>/dev/null; then
        [ -e /proc/sys/fs/binfmt_misc/rosetta ] || printf '%s' '{rosetta}' > /proc/sys/fs/binfmt_misc/register || true
      fi
      exit 0
  - path: /usr/local/sbin/patina-port
    permissions: "0755"
    content: |
      #!/bin/sh
      # First line from the Mac names the port; the rest is the connection.
      read port
      case "$port" in ''|*[!0-9]*) exit 1;; esac
      exec socat {HALF_CLOSE} - TCP:127.0.0.1:"$port"
  - path: /etc/systemd/system/patina-boot.service
    content: |
      [Unit]
      Description=Patina shares from the Mac
      After=local-fs.target
      Before=docker.service
      [Service]
      Type=oneshot
      RemainAfterExit=yes
      ExecStart=/usr/local/sbin/patina-boot
      [Install]
      WantedBy=multi-user.target
  - path: /etc/systemd/system/patina-docker.service
    content: |
      [Unit]
      Description=Patina Docker API channel
      After=docker.service
      Wants=docker.service
      [Service]
      KillMode=process
      ExecStart=/usr/bin/socat {HALF_CLOSE} VSOCK-LISTEN:{PORT_DOCKER},reuseaddr,fork UNIX-CONNECT:/run/docker.sock
      Restart=always
      RestartSec=1
      [Install]
      WantedBy=multi-user.target
  - path: /etc/systemd/system/patina-ports.service
    content: |
      [Unit]
      Description=Patina port forwarding channel
      [Service]
      KillMode=process
      ExecStart=/usr/bin/socat {HALF_CLOSE} VSOCK-LISTEN:{PORT_FORWARD},reuseaddr,fork EXEC:/usr/local/sbin/patina-port
      Restart=always
      RestartSec=1
      [Install]
      WantedBy=multi-user.target
  - path: /usr/local/sbin/patina-memory
    permissions: "0755"
    content: |
{memory_script}  - path: /etc/systemd/system/patina-memory.service
    content: |
{memory_unit}  - path: /etc/docker/daemon.json
    content: |
      {{ "features": {{ "buildkit": true }} }}
  - path: /etc/default/grub.d/90-patina.cfg
    content: |
      GRUB_CMDLINE_LINUX_DEFAULT="{KERNEL_ARGS}"
      GRUB_TIMEOUT=0
runcmd:
  - [systemctl, daemon-reload]
  - [systemctl, enable, --now, patina-boot.service]
  - [systemctl, enable, --now, docker.service]
  - [systemctl, enable, --now, patina-docker.service, patina-ports.service, patina-memory.service]
  - [sh, -c, "update-grub || true"]
  - [sh, -c, "touch {marker}"]
"#
    )
}

/// Bring a VM set up by an older Patina up to date. Each step checks first,
/// so this does nothing on a current VM. Restarting the channels keeps their
/// open connections (`KillMode=process`), including this one.
#[cfg(target_os = "macos")]
pub async fn upgrade(d: &bollard::Docker) -> Result<bool, String> {
    let out = super::kube::in_vm(d, &upgrade_script()).await?;
    Ok(out.contains("upgraded"))
}

#[cfg(any(target_os = "macos", test))]
fn upgrade_script() -> String {
    format!(
        r##"set -e
units=
for u in patina-docker patina-ports; do
  f=/etc/systemd/system/$u.service
  [ -f "$f" ] || continue
  grep -q -- 'socat {HALF_CLOSE} ' "$f" && continue
  sed -i -e 's#^ExecStart=/usr/bin/socat VSOCK-LISTEN#ExecStart=/usr/bin/socat {HALF_CLOSE} VSOCK-LISTEN#' -e 's#^\[Service\]$#[Service]\nKillMode=process#' "$f"
  units="$units $u.service"
done
p=/usr/local/sbin/patina-port
if [ -f "$p" ] && ! grep -q -- 'socat {HALF_CLOSE} ' "$p"; then
  sed -i 's#exec socat - TCP#exec socat {HALF_CLOSE} - TCP#' "$p"
fi
memory() {{
  cat <<'PATINA'
{MEMORY_SCRIPT}PATINA
}}
if [ "$(cat /usr/local/sbin/patina-memory 2>/dev/null)" != "$(memory)" ]; then
  memory > /usr/local/sbin/patina-memory
  chmod 755 /usr/local/sbin/patina-memory
  echo upgraded
fi
if [ ! -f /etc/systemd/system/patina-memory.service ]; then
  cat > /etc/systemd/system/patina-memory.service <<'PATINA'
{memory_unit}PATINA
  systemctl daemon-reload
  systemctl enable --now patina-memory.service
  echo upgraded
fi
# The kernel's command line, for the balloon. It takes effect when the VM next starts.
e=/boot/efi/EFI/debian/grub.cfg
if [ -f "$e" ] && ! grep -q 'init_on_alloc=0' "$e"; then
  sed -i 's#^\(  linux .* console=hvc0\)$#\1 init_on_alloc=0#' "$e"
  echo upgraded
fi
g=/etc/default/grub.d/90-patina.cfg
if [ -f "$g" ] && ! grep -q 'init_on_alloc=0' "$g"; then
  sed -i 's#^GRUB_CMDLINE_LINUX_DEFAULT="console=hvc0"$#GRUB_CMDLINE_LINUX_DEFAULT="{KERNEL_ARGS}"#' "$g"
  update-grub >/dev/null 2>&1 || true
  echo upgraded
fi
if [ -n "$units" ]; then
  systemctl daemon-reload
  systemd-run --quiet --on-active=1 systemctl restart $units
  echo upgraded
fi
"##,
        memory_unit = memory_unit(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_is_cloud_config() {
        let u = user_data("/Users/someone");
        assert!(u.starts_with("#cloud-config\n"));
        assert!(u.contains("mount -t virtiofs patina-home '/Users/someone'"));
        assert!(u.contains("socat -t 86400 VSOCK-LISTEN:2375"));
        assert!(u.contains("exec socat -t 86400 - TCP:127.0.0.1"));
        assert_eq!(u.matches("KillMode=process").count(), 2);
        assert!(u.contains("touch '/Users/someone/.patina/vm/provisioned'"));
        assert!(!u.contains("{{"));
    }

    #[test]
    fn user_data_sets_up_the_memory_channel() {
        let u = user_data("/Users/someone");
        assert!(u.contains("\n      ExecStart=/usr/bin/socat VSOCK-LISTEN:2377,reuseaddr,fork EXEC:/usr/local/sbin/patina-memory\n"));
        assert!(u.contains("\n        echo \"BalloonFeatures: $(cat \"$f\")\"\n"));
        assert!(u.contains("\n        *\" init_on_alloc=0 \"*) echo \"InitOnAlloc: 0\" ;;\n"));
        assert!(u.contains("\n      WantedBy=multi-user.target\n  - path: /etc/docker/daemon.json\n"));
        assert!(u.contains("patina-ports.service, patina-memory.service]"));
        assert!(u.contains("\n      GRUB_CMDLINE_LINUX_DEFAULT=\"console=hvc0 init_on_alloc=0\"\n"));
    }

    #[test]
    fn upgrade_matches_what_new_vms_get() {
        let s = upgrade_script();
        assert!(s.contains("ExecStart=/usr/bin/socat -t 86400 VSOCK-LISTEN#"));
        assert!(s.contains("exec socat -t 86400 - TCP#"));
        assert!(s.contains("s#^\\[Service\\]$#[Service]\\nKillMode=process#"));
        assert!(s.contains(&format!("memory() {{\n  cat <<'PATINA'\n{MEMORY_SCRIPT}PATINA\n}}\n")));
        assert!(s.contains(&format!("<<'PATINA'\n{}PATINA\n", memory_unit())));
        assert!(s.contains("systemctl enable --now patina-memory.service"));
        assert!(s.contains("s#^\\(  linux .* console=hvc0\\)$#\\1 init_on_alloc=0#"));
        assert!(s.contains("#GRUB_CMDLINE_LINUX_DEFAULT=\"console=hvc0 init_on_alloc=0\"#"));
        assert!(!s.contains("{{"));
    }

    #[test]
    fn upgrade_installs_the_memory_script_as_is() {
        // Run the script's first step against a scratch file in place of
        // /usr/local/sbin/patina-memory.
        let dir = std::env::temp_dir().join(format!("patina-guest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("patina-memory");
        let s = upgrade_script();
        let start = s.find("memory() {").unwrap();
        let end = s[start..].find("\nif [ ! -f /etc/systemd/system/patina-memory.service ]").unwrap() + start;
        let step = s[start..end].replace("/usr/local/sbin/patina-memory", &path.to_string_lossy());
        for want in ["upgraded\n", ""] {
            let out = std::process::Command::new("/bin/sh").arg("-c").arg(&step).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), want);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), MEMORY_SCRIPT);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn indents_lines_for_yaml() {
        assert_eq!(indent("a\n\n  b\n", 2), "  a\n\n    b\n");
    }

    #[test]
    fn quotes_paths_for_the_shell() {
        assert_eq!(sh_quote("/Users/o'neil"), "'/Users/o'\\''neil'");
    }
}
