# Contributing to Patina

Patina is a Rust app built with [egui/eframe](https://github.com/emilk/egui) from the *Patina
desktop UI* design: the same layout, colors, type, copy and motion. This page covers running it
from source, how it works, sending a change, and making a release. To install it instead, see
the [README](README.md).

## Run it from source

You need [Rust](https://rustup.rs) (stable), and either a Mac with Apple silicon and macOS 13 or
later, or Linux with [Docker Engine](https://docs.docker.com/engine/install/).

```sh
cargo run --release
cargo test
```

The first launch walks through setup. After that, the app opens to the main window and starts the
engine. You can turn that off in Settings › General.

To build a double-clickable app into `target/Patina.app`:

```sh
./scripts/bundle-macos.sh
```

### Without an engine

`--demo` runs the simulated engine from the design instead, with nothing touching Docker. It's
the quickest way to work on the UI:

```sh
cargo run -- --demo
cargo run -- --scenario trouble            # containers needing attention (simulated)
cargo run -- --show run --phase conflict   # run dialog, port in use (simulated)
cargo run -- --popover --scenario failed  # the menu bar popover in a window (simulated)
cargo run -- --help                        # every option
```

### When something goes wrong

Set `PATINA_DEBUG=1` to print Docker API errors. The engine's log is
`~/.patina/logs/engine.log`, and the guest's boot log is `~/.patina/logs/console.log`.

## Send a change

1. Fork the repository and branch from `main`.
2. Run `cargo test`, and keep `cargo clippy` free of new warnings. Match the formatting of the
   code around your change: the code isn't formatted with `cargo fmt`'s defaults, so running it
   rewrites whole files.
3. Try the change where it runs. `--demo` covers most UI work. Engine changes need the real
   thing: macOS for Patina's VM, and Linux for Docker on the computer (`engine::NATIVE`).
4. Open a pull request against `main` that says what changed and how you tested it.

## The command line

The engine has a command line, as well as the app:

```sh
patina engine setup     # download and prepare everything
patina engine start     # boot the VM in the background
patina engine status    # starting | booting | installing | running | stopping
patina engine stop      # shut the guest down cleanly
patina engine reset     # delete the VM disk (keeps downloads); run setup again after
patina engine kube on   # Kubernetes (k3s) in the VM; also off | reset
patina engine df        # what takes up the disk, and what's reclaimable
patina engine clean     # remove what nothing uses (see below)
```

`patina engine clean` takes the kinds to remove: `dangling`, `images`, `distros` (machine base
images no machine uses), `cache`, `networks`, `stopped` (containers), `volumes`, or `all`. With
none, it removes everything unused except stopped containers and volumes.

Linux machines have their own:

```sh
patina machine create               # ubuntu, named ubuntu (or ubuntu-2, ... when taken)
patina machine create fedora dev    # or debian, arch, alpine, with a name of your choice
patina machine shell ubuntu         # a login shell as your user
patina machine list                 # names, distros, addresses and URLs
patina machine stop ubuntu          # also start, delete
```

On Linux, `engine start` and `engine status` check the computer's Docker Engine, and the commands
that need Patina's VM say so instead.

## How the engine works

```
 Patina.app ──┐                       ┌── Linux VM (Debian 13, Virtualization.framework) ──┐
              │ ~/.patina/run/        │                                                     │
 docker CLI ──┼─ docker.sock ── vsock 2375 ── socat ── /run/docker.sock ── Docker Engine     │
              │                       │                                                     │
 localhost:N ─┼─ port forwarder ─ vsock 2376 ── socat ── 127.0.0.1:N (published ports)      │
              │                       │                                                     │
 ~/ (home) ───┼──────── virtiofs ─────┼── mounted at the same path, so -v ~/code:/src works │
 Rosetta ─────┼──────── virtiofs ─────┼── registered with binfmt_misc for linux/amd64       │
              └───────────────────────┴─────────────────────────────────────────────────────┘
```

- **Setup** (first launch, or `patina engine setup`) downloads Debian's official arm64 cloud image
  and checks it against Debian's published SHA-512. It grows the image into a 64 GB sparse disk
  (only used space takes room), points the boot loader at the virtio console, and writes a
  first-boot seed. It also installs the `docker` and `docker-compose` commands into
  `~/.patina/bin` and adds a `patina` docker context, without switching your current one.
- **First boot** installs Docker Engine in the guest with cloud-init (about 2–3 minutes).
  Later boots take about 4 seconds; shutting down takes about 2.
- **The docker command uses Patina.** When the engine finishes starting, Patina makes `patina`
  the current docker context. It also adds one marked line to `~/.zprofile` (and
  `~/.bash_profile`, if you have one) that puts `~/.patina/bin` first on PATH in new terminals.
  Both are switches in Settings › General; turning them off undoes them.
- **The helper** (`patina engine run`) owns the VM. It keeps running after the app quits, like a
  system service. To get the virtualization entitlement, the app keeps an ad-hoc-signed copy of
  itself in `~/.patina/bin/patina-engine`.

Everything lives in `~/.patina`: `vm/` (disk, EFI variables, seed), `bin/` (docker,
docker-compose, the helper), `run/` (sockets), `logs/` (`engine.log`, `console.log`), and
`config.json`.

### Linux machines

A machine is a full Linux system next to your containers, sharing the VM's kernel. Each one boots
its distro's own init (systemd, or OpenRC on Alpine), so `systemctl`, services and `sudo` work as
they do on a server. It has a user with your Mac name and uid and passwordless sudo, a home of
its own in `/home`, and your Mac home folder at the same path, as in containers. Its files last
until you delete it, and machines that were running start again with the engine.

**Addresses.** Each machine gets a fixed address on the network macOS gives Patina's VM
(`192.168.64.192`–`.254` by default, skipping addresses macOS has lent to other VMs). Your Mac
reaches it directly on any port, so a server in a machine answers at `http://192.168.64.254`,
port 80 included. The engine also names running machines `<name>.patina.local` on this Mac only
(nothing is announced on your network), so `http://ubuntu.patina.local` works in a browser. The
card shows the URL and address; the info button shows them with the user, mounts, shell command,
memory and disk. Underneath it's an ipvlan network, `patina-machines`, that shares the VM's MAC
address; the machine's default route stays on Docker's bridge, so it reaches published container
ports at `172.17.0.1`.

The first machine of a distro takes about a minute, and its card shows how far along it is: the
download in megabytes, then each package as it installs. Patina starts the distro's official image,
installs init and sudo, adds your user, and keeps the result as a base image
(`patina.local/ubuntu:24.04` in Images). Later machines of that distro start in a few seconds.
Arch Linux has no arm64 image on Docker Hub, so its root filesystem comes from
[images.linuxcontainers.org](https://images.linuxcontainers.org), checked against the SHA-256
published with it. Underneath, a machine is a privileged Docker container labelled
`dev.patina.machine`, so `docker exec -it ubuntu su - $USER` works too. The Containers view
leaves machines out.

### On Linux

Linux runs containers natively, so Patina doesn't boot a VM there. `engine::NATIVE` (true
everywhere but macOS) routes the engine's status, start and stop to `src/engine/host.rs`, and the
rest of the app is the same. It finds the socket from `DOCKER_HOST`, then rootless Docker
(`$XDG_RUNTIME_DIR/docker.sock`), then `/var/run/docker.sock`. When Docker doesn't answer,
Patina says why: it isn't installed, you're not in the `docker` group, or it isn't running.

Open in Terminal uses the desktop's default terminal or one chosen in Settings › General (GNOME
Terminal, Ptyxis, Konsole, kitty, Alacritty, WezTerm, Ghostty and others). Machines and
Kubernetes run scripts in PID 1's namespaces through a privileged container: in Patina's VM
that's the VM, but with the computer's own Docker it would be the computer itself, as root. So
on Linux they stop with a message before touching anything.

## Code map

| File | What it holds |
| --- | --- |
| `src/engine/vm.rs` | The VM: Virtualization.framework through objc2 (EFI boot, disks, NAT, virtio console, vsock, virtiofs, Rosetta) |
| `src/engine/daemon.rs` | `patina engine run`: Docker socket proxy, port forwarding, status and stop |
| `src/engine/setup.rs` | Downloads, checksum check, disk and boot loader, the docker CLI |
| `src/engine/guest.rs` | The guest's first-boot provisioning (cloud-init) |
| `src/engine/control.rs` | The app's side: start, stop, status |
| `src/engine/host.rs` | On Linux: this computer's Docker Engine, its socket, and why it isn't answering |
| `src/engine/machines.rs` | Linux machines: base images, setup, addresses, create, start, stop |
| `src/engine/cleanup.rs` | Disk usage by kind, and Clean up |
| `src/ui/dashboard.rs` | The dashboard |
| `src/engine/mdns.rs` | `<machine>.patina.local` names on this Mac (mDNSResponder, local only) |
| `src/terminal.rs` | Opening shells in your terminal app |
| `src/engine/kube.rs` | Kubernetes with k3s, and running scripts in the VM |
| `src/live.rs` | The app's Docker API client (bollard): polling, stats and log streams, actions |
| `src/sim.rs` | App state and every action; the simulated engine for `--demo` |
| `src/ui/`, `src/onboarding.rs` | The views and the first-run flow |
| `src/menubar.rs` | The status item (AppKit through objc2), its right-click menu, the Dock icon |
| `src/widgets.rs`, `src/tween.rs`, `src/icons.rs`, `src/theme.rs` | Drawing, animation, icons, colors and fonts |
| `install.sh` | The Linux installer from the README |
| `scripts/` | Building the app bundle, the DMG, and the Linux packages |

## Make a release

Set `version` in `Cargo.toml` (`cargo build` updates `Cargo.lock`), commit, then tag that commit
and push both:

```sh
git tag v1.1.0
git push origin main v1.1.0
```

The tag starts the Release workflow (`.github/workflows/release.yml`). It checks that the tag
matches `Cargo.toml` and runs the tests on Apple silicon, and builds the AppImages and .debs on
Linux (x86_64 and aarch64). Then it publishes a GitHub release with the commits since the last
tag, those files, and their `SHA256SUMS`, which `install.sh` checks downloads against. As soon as
the macOS tests pass, it points the tap's `Formula/patina.rb` at the new tag and its commit, so
`brew update && brew upgrade patina` picks it up. A tag with a suffix (`v1.1.0-beta.1`) makes a
pre-release and leaves the formula alone. **Actions › Release › Run workflow** releases an
existing tag again.

### The Homebrew tap

The formula lives in [caffeines/homebrew-patina](https://github.com/caffeines/homebrew-patina).
It builds Patina from source on the Mac it's installed on, which needs no Apple Developer ID or
notarization: Gatekeeper checks apps downloaded from the internet, and this one is compiled
locally.

To update the tap, the workflow needs a `HOMEBREW_TAP_TOKEN` secret: a fine-grained token with
Contents read and write on caffeines/homebrew-patina. Without it the Homebrew job fails; add the
secret and re-run the job, or set the tag and its commit (`git rev-parse v1.1.0`) in the formula
by hand:

```ruby
url "https://github.com/caffeines/patina.git",
    tag:      "v1.1.0",
    revision: "<the commit from git rev-parse>"
```

The revision pins the build to that commit even if the tag is moved.

### A DMG

Releases don't include a DMG; Homebrew is how Patina is installed on macOS. To build one by
hand, `./scripts/release-macos.sh` builds `target/dist/Patina-<version>-arm64.dmg`. Without the
variables below, the DMG gets an ad-hoc signature, which works on this Mac but other Macs block.

To publish one that opens cleanly on any Mac:

1. **Get a Developer ID Application certificate.** It comes with the paid Apple Developer
   Program; the account holder creates it in Xcode › Settings › Accounts › Manage
   Certificates › +. (An "Apple Development" certificate can't sign for distribution.)
2. **Save notarization credentials once.** Use an app-specific password from
   account.apple.com:
   ```sh
   xcrun notarytool store-credentials patina-notary \
     --apple-id you@example.com --team-id TEAMID --password app-specific-password
   ```
3. **Build, sign and notarize:**
   ```sh
   DEVELOPER_ID="Developer ID Application: Your Name (TEAMID)" \
   NOTARY_PROFILE=patina-notary \
   BUNDLE_ID=com.yourname.patina \
   ./scripts/release-macos.sh
   ```
   The script signs the app (hardened runtime plus the virtualization entitlement in
   `scripts/Patina.entitlements`), builds the DMG, signs it, sends it to Apple, and staples the
   ticket.
4. **Publish it.** For example, attach it to a GitHub release with
   `gh release create v0.1.0 target/dist/Patina-0.1.0-arm64.dmg`.

The DMG is for Apple silicon and macOS 13 or later.

### Linux packages

`./scripts/release-linux.sh` builds `Patina-<version>-<arch>.AppImage` and
`patina_<version>_<debarch>.deb` into `target/dist`, for the architecture it runs on. It
downloads a pinned appimagetool and AppImage runtime, and checks them against their SHA-256. The
.deb depends on the glibc version the binary needs, so build on the oldest system you want to
support: the workflow uses Ubuntu 22.04.

## Fonts

Instrument Sans and DM Mono (Latin subsets, via Fontsource) are licensed under the SIL Open Font
License 1.1.
