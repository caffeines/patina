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

pub mod config;
pub mod control;
pub mod guest;
pub mod paths;
pub mod setup;

#[cfg(target_os = "macos")]
pub mod daemon;
#[cfg(target_os = "macos")]
mod vm;

/// vsock port where the guest serves the Docker API.
pub const PORT_DOCKER: u32 = 2375;
/// vsock port where the guest forwards a TCP port named in the first line.
pub const PORT_FORWARD: u32 = 2376;
