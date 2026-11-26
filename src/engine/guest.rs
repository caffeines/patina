//! What the guest does on first boot (cloud-init NoCloud seed).
//!
//! The VM is a stock Debian cloud image. On first boot cloud-init installs
//! Docker Engine and socat, and sets up three small services:
//!
//! - `patina-boot` mounts the Mac's home folder at the same path (so bind
//!   mounts like `-v ~/code:/src` work) and registers Rosetta for amd64.
//! - `patina-docker` serves `/run/docker.sock` on vsock port 2375.
//! - `patina-ports` connects vsock port 2376 to a guest TCP port named in
//!   the first line, which is how published ports reach localhost on the Mac.
//!
//! socat runs with a long `-t`: by default it closes a connection half a
//! second after one side finishes sending, which cut off `docker run` and
//! `docker exec` output once the docker command had sent all its input.
//! [`upgrade`] brings VMs set up before that up to date.

use bollard::Docker;

use super::{kube, PORT_DOCKER, PORT_FORWARD};

/// How long socat keeps a connection open after one side finishes sending:
/// long enough for any command's output.
const HALF_CLOSE: &str = "-t 86400";

/// Single-quote a path for /bin/sh.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn meta_data() -> String {
    "instance-id: patina-engine-1\nlocal-hostname: patina\n".to_string()
}

pub fn user_data(home: &str) -> String {
    let h = sh_quote(home);
    let marker = sh_quote(&format!("{home}/.patina/vm/provisioned"));
    // binfmt_misc entry for x86_64 ELF binaries, as documented for Rosetta in Linux VMs.
    let rosetta = r":rosetta:M::\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x3e\x00:\xff\xff\xff\xff\xff\xfe\xfe\x00\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\xff:/mnt/rosetta/rosetta:OCF";
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
  - path: /etc/docker/daemon.json
    content: |
      {{ "features": {{ "buildkit": true }} }}
  - path: /etc/default/grub.d/90-patina.cfg
    content: |
      GRUB_CMDLINE_LINUX_DEFAULT="console=hvc0"
      GRUB_TIMEOUT=0
runcmd:
  - [systemctl, daemon-reload]
  - [systemctl, enable, --now, patina-boot.service]
  - [systemctl, enable, --now, docker.service]
  - [systemctl, enable, --now, patina-docker.service, patina-ports.service]
  - [sh, -c, "update-grub || true"]
  - [sh, -c, "touch {marker}"]
"#
    )
}

/// Bring a VM set up by an older Patina up to date. Each step checks first,
/// so this does nothing on a current VM. Restarting the channels keeps their
/// open connections (`KillMode=process`), including this one.
pub async fn upgrade(d: &Docker) -> Result<bool, String> {
    let out = kube::in_vm(d, &upgrade_script()).await?;
    Ok(out.contains("upgraded"))
}

fn upgrade_script() -> String {
    format!(
        r#"set -e
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
if [ -n "$units" ]; then
  systemctl daemon-reload
  systemd-run --quiet --on-active=1 systemctl restart $units
  echo upgraded
fi
"#
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
    fn upgrade_matches_what_new_vms_get() {
        let s = upgrade_script();
        assert!(s.contains("ExecStart=/usr/bin/socat -t 86400 VSOCK-LISTEN#"));
        assert!(s.contains("exec socat -t 86400 - TCP#"));
        assert!(s.contains("s#^\\[Service\\]$#[Service]\\nKillMode=process#"));
        assert!(!s.contains("{{"));
    }

    #[test]
    fn quotes_paths_for_the_shell() {
        assert_eq!(sh_quote("/Users/o'neil"), "'/Users/o'\\''neil'");
    }
}
