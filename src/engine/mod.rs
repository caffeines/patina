//! Patina's own container engine.
//!
//! Patina runs Docker Engine inside a small Linux VM that it boots itself
//! with Apple's Virtualization framework. Nothing here depends on OrbStack,
//! Docker Desktop or any other app:
//!
//! - [`setup`] downloads a Debian cloud image (checksum-verified), turns it
//!   into the VM's disk, writes the first-boot provisioning, and installs the
//!   `docker` and `docker-compose` commands into `~/.patina/bin`.
//! - [`daemon`] is the `patina engine run` helper process. It boots the VM,
//!   serves the Docker API at `~/.patina/run/docker.sock` over a private
//!   virtio socket, forwards published container ports to localhost, and
//!   answers status and stop requests on `~/.patina/run/engine.sock`.
//! - [`control`] is the app's side: start, stop and status.
//! - [`machines`] runs Linux machines: system containers that boot their
//!   distro's own init.
//!
//! On Linux there's no VM: Docker runs natively, so Patina uses the Docker
//! Engine already on the computer ([`host`]) and [`NATIVE`] is true.

pub mod cleanup;
pub mod config;
pub mod control;
pub mod guest;
pub mod host;
pub mod integrate;
pub mod kube;
pub mod machines;
pub mod migrate;
pub mod paths;
pub mod setup;

#[cfg(target_os = "macos")]
pub mod daemon;
#[cfg(target_os = "macos")]
mod mdns;
#[cfg(target_os = "macos")]
mod vm;

/// Patina uses this computer's own Docker Engine instead of booting a VM:
/// everywhere but macOS. Patina doesn't own that engine, so it doesn't set it
/// up, start, stop or reset it.
pub const NATIVE: bool = cfg!(not(target_os = "macos"));

/// vsock port where the guest serves the Docker API.
pub const PORT_DOCKER: u32 = 2375;
/// vsock port where the guest forwards a TCP port named in the first line.
pub const PORT_FORWARD: u32 = 2376;
