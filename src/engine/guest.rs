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

use super::{PORT_DOCKER, PORT_FORWARD};

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
      exec socat - TCP:127.0.0.1:"$port"
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
      ExecStart=/usr/bin/socat VSOCK-LISTEN:{PORT_DOCKER},reuseaddr,fork UNIX-CONNECT:/run/docker.sock
      Restart=always
      RestartSec=1
      [Install]
      WantedBy=multi-user.target
  - path: /etc/systemd/system/patina-ports.service
    content: |
      [Unit]
      Description=Patina port forwarding channel
      [Service]
      ExecStart=/usr/bin/socat VSOCK-LISTEN:{PORT_FORWARD},reuseaddr,fork EXEC:/usr/local/sbin/patina-port
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_is_cloud_config() {
        let u = user_data("/Users/someone");
        assert!(u.starts_with("#cloud-config\n"));
        assert!(u.contains("mount -t virtiofs patina-home '/Users/someone'"));
        assert!(u.contains("VSOCK-LISTEN:2375"));
        assert!(u.contains("touch '/Users/someone/.patina/vm/provisioned'"));
        assert!(!u.contains("{{"));
    }

    #[test]
    fn quotes_paths_for_the_shell() {
        assert_eq!(sh_quote("/Users/o'neil"), "'/Users/o'\\''neil'");
    }
}
