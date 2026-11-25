# Patina

A native desktop app for containers and Linux machines, written in Rust with
[egui/eframe](https://github.com/emilk/egui). It is built from the *Patina desktop UI*
design: the same layout, colors, type, copy and motion.

The engine is **simulated**, just like in the design prototype. Containers start and stop, CPU and
memory move, logs stream, images pull layer by layer and Kubernetes boots its pods. None of it
touches Docker on this machine.

## Run

```sh
cargo run --release
```

The first launch shows the first-run flow. After that, the app opens straight to the main window.

Build a double-clickable macOS app (with icon) into `target/Patina.app`:

```sh
./scripts/bundle-macos.sh
```

Run the engine tests:

```sh
cargo test
```

## What's in it

- **Containers:** grouped by Compose project, with live CPU sparklines, filtering and search.
  Start, stop, pause, restart and delete (with undo). A row menu, plus a detail panel with logs,
  a terminal and details.
- **Images, Volumes, Networks:** size bars, which container uses each one, pull with per-layer
  progress, prune unused images, create volumes and networks. Deleting something that is still in
  use is blocked.
- **Machines:** Ubuntu, Debian, Fedora, Arch or Alpine machines with a small live shell.
- **Kubernetes:** a cluster switch, a staged boot, and the kube-system pods.
- **Settings:** General, Resources (CPU and memory sliders with "Apply and restart"), Network,
  Engine and About.
- **Engine card:** boot progress ring, memory use, Restart and Start/Stop.
- **Command palette:** `⌘K` (or `Ctrl K`). Use ↑ ↓ to move, Enter to run, Esc to close.
- **Cases:** engine stopped, engine failed (with boot log), loading skeleton, empty states, no
  search matches, update banner, disk-almost-full banner, registry unreachable.

## Open a specific design board

Each board in the design can be opened from the command line:

```sh
cargo run -- --scenario trouble            # containers needing attention
cargo run -- --show run --phase conflict   # run dialog, port already in use
cargo run -- --view images --show pull     # pull in progress
cargo run -- --view settings --pane resources --phase dirty
cargo run -- --onboarding --step 3         # first run, "Bring your work"
cargo run -- --help                        # every option
```

## Code map

| File | What it holds |
| --- | --- |
| `src/sim.rs` | The engine model and every action (start, stop, pull, prune and the rest). It's a port of the prototype's logic, with tests. |
| `src/ui/` | One module per view: `side`, `main_view`, `containers`, `assets`, `machines`, `kube`, `settings`, `overlays` |
| `src/onboarding.rs` | The 4-step first-run flow |
| `src/widgets.rs` | Buttons, switches, segmented controls, status dots, rings, sheen, shimmer |
| `src/tween.rs` | CSS-style transitions with cubic-bezier easing |
| `src/icons.rs` | The design's SVG icon paths |
| `src/theme.rs` | Colors and fonts |

To connect a real Docker engine, replace the methods on `State` in `sim.rs`. The views only call
those methods.

The menu bar popover from the design is not built yet.

## Fonts

Instrument Sans and DM Mono (Latin subsets, via Fontsource) are licensed under the SIL Open Font
License 1.1.
