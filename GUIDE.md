# memreduct guide

A practical reference for `memreduct`, the Rust port of Henry++ Mem Reduct
for Linux. For agent/build internals see [AGENTS.md](AGENTS.md); for release
history see [CHANGELOG.md](CHANGELOG.md).

## Philosophy

No daemon config files, no GUI, no magic: every capability is a subcommand,
every subcommand speaks JSON (`--json`), so you compose pipelines with jq,
cron, systemd, or your own scripts. The binary is fully static (musl) in
releases and runs unchanged across glibc/musl distros on x86_64 and aarch64.

## Install

### Static release binaries (any distro)

Grab `memreduct-vX.Y.Z-<target>.tar.gz` from
[Releases](https://github.com/AkumaNomu/MemReRust/releases), untar, copy to
`/usr/local/bin`. Targets: `x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`. GUI builds are `memreduct-gui-*` tarballs (same binary, built with `--features gui`).

### From source

```sh
cargo install --git https://github.com/AkumaNomu/MemReRust                 # CLI only
cargo install --features gui --git https://github.com/AkumaNomu/MemReRust  # with native GUI (needs Wayland/X11 dev libs on Linux)
cargo build --features gui --release && ./target/release/memreduct gui     # run the GUI
```

Distro build deps:

| Distro | Packages |
| --- | --- |
| Debian/Ubuntu | `build-essential cargo` |
| Fedora | `rust cargo` (`dnf group install development-tools`) |
| Arch | base-devel + `rust` |
| Alpine | `cargo rust` |
| openSUSE | `rust cargo gcc` |

Packaging sources live in `packaging/`: Arch `PKGBUILD`, Alpine `APKBUILD`,
Fedora `.spec`, Debian dir, plus systemd unit examples under
`packaging/systemd/`.

### Shell completions

```sh
memreduct completions bash | sudo tee /usr/share/bash-completion/completions/memreduct >/dev/null
memreduct completions zsh | sudo tee /usr/share/zsh/site-functions/_memreduct >/dev/null
memreduct completions fish | sudo tee /usr/share/fish/vendor_completions.d/memreduct.fish >/dev/null
```

### GUI (Windows + Linux, `gui` feature)

```sh
memreduct gui        # or
memreduct --gui      # or
memreduct            # no args → GUI if built with --features gui
```

Compact native window (egui, ~15MB, dark): live RAM/swap bars + PSI + history graph, `Clean`/`Compact` buttons, `Auto-clean` toggle, and tabs for **Processes** / **OOM** / **Slab** / **Zram** / **Cgroup** / **Leak Scan** — every CLI feature is reachable without opening a terminal. Details and platform differences are in [DOCUMENTATION.md](DOCUMENTATION.md#use-the-compact-gui-windows--linux).

## Command reference

### `status`

Snapshot of RAM, swap, cache, and PSI pressure. Works unprivileged.

```sh
$ memreduct status
Memory
  total:      14.9 GiB
  used:       9.2 GiB (61%)
  free:       687.0 MiB
  available:  5.7 GiB
  cached:     6.1 GiB
  buffers:    576.0 KiB
  reclaimable: 381.0 MiB
  swap:       6.7 GiB / 12.0 GiB
  pressure:   some 0.00% / full 0.00% (avg10)
```

"Used" follows the kernel's own definition: `MemTotal - MemAvailable`
(available already discounts reclaimable caches). `pressure` needs kernel
4.20+ with PSI enabled (`CONFIG_PSI=y`, not disabled via boot flag); it shows
as `unavailable` otherwise.

### `clean`

Drop clean page cache and/or reclaimable slab through
`/proc/sys/vm/drop_caches`. Runs `sync()` first. Root required.

```sh
sudo memreduct clean                     # all caches
sudo memreduct clean --mode page-cache   # value 1
sudo memreduct clean --mode slab         # value 2
memreduct clean --dry-run                # no-op preview
```

Honest advice (same as upstream): Linux reclaims caches automatically.
Forced drops mainly help reproducible benchmarking or testing OOM paths;
routine "cleaning" usually *hurts* I/O performance by throwing out hot cache.

### `watch`

Auto-clean loop. Two engines:

- **Polling** (default): reads `/proc/meminfo` every `--interval SECS`,
  cleans when used% ≥ `--threshold`.
- **PSI triggers** (`--psi some|full`): registers a kernel trigger on
  `/proc/pressure/memory` and blocks in `poll()` — near-zero CPU between
  events. Requires root (write access to the pressure file) and kernel 4.20+.

Extra knobs (both engines):

| Flag | Meaning |
| --- | --- |
| `--swap-threshold PCT` | Also trip when swap-used ≥ PCT% (0 = off, polling only) |
| `--cooldown SECS` | Minimum seconds between automatic cleans (default 300; 0 = disable) |
| `--exec CMD` | Shell command run after each successful triggered clean |

The default `--cooldown 300` prevents thrash loops where cleaning frees
little and pressure returns immediately; during cooldown windows the loop
keeps monitoring silently. `--exec 'notify-send ...'` is the intended hook
for desktop notifications.

```sh
# Desktop-friendly polling watcher
sudo memreduct watch --threshold 92 --interval 15 --swap-threshold 70 \
    --exec 'notify-send -u critical memreduct "cleaned at $(date +%T)"'

# Event-driven PSI watcher (root)
sudo memreduct watch --psi some --threshold 8 --mode page-cache

# One-shot check for cron/systemd timers
memreduct watch --once --threshold 90 --dry-run
```

### Systemd integration

Two supported shapes (examples in `packaging/systemd/`):

1. **Resident service**: run `watch` as a hardened system service
   (`memreduct-watch.service.example`). Best when you want PSI-triggered
   reaction times.
2. **Timer + oneshot**: `memreduct-clean.timer.example` fires
   `memreduct watch --once` periodically. More idiomatic systemd, zero
   resident memory, but polling granularity only.

### `compact`

Ask the kernel to compact physical memory zones (fight fragmentation so
higher-order allocations succeed). Writes `/proc/sys/vm/compact_memory`;
root + `CONFIG_COMPACTION` required.

```sh
sudo memreduct compact            # reports MemFree before -> after
sudo memreduct compact --dry-run
```

Compaction *relocates* pages to create contiguous free ranges — it does not
release memory, so treat the MemFree delta as informational. The write blocks
until kernel compaction finishes; on very large systems this can take a
while. This is the closest analog to Windows working-set trimming that Linux
offers safely.

### `pss`

Per-process proportional set size from `smaps_rollup` (kernel 4.14+).
PSS splits shared pages across sharers — far more honest than RSS.

```sh
memreduct pss                 # top 10 by PSS (your visible processes)
sudo memreduct pss --top 20   # system-wide view
memreduct pss 1234 5678       # specific PIDs
```

### `oom`

Rank processes by OOM-killer score (what the kernel would kill first) plus
the system-wide oom_kill event count since boot. Cheap: uses `statm`, not
smaps walks.

```sh
$ sudo memreduct oom --top 5
kernel OOM kills since boot: 0
     PID    SCORE   ADJ        RSS  COMM
     913      809   200  294.3 MiB  ai.opencode.des
...
```

`ADJ` is `oom_score_adj`; anything pinned at `-1000` is unkillable.

### `grow`

Leak detector: two `smaps_rollup` sweeps `--interval SECS` apart, matched by
PID + process starttime (so PID reuse can't fake growth), reporting ΔPSS ≥
`--min`.

```sh
sudo memreduct grow --interval 120 --min 16M
```

### `slab`

Top kernel slab caches from `/proc/slabinfo` — where kernel object memory
goes (inodes, dentries, conntrack, kmalloc buckets). Readable by root only
on most modern kernels.

```sh
$ sudo memreduct slab --top 5
slab total 1.2 GiB (top 5 of 93 caches)
  CACHE                            SIZE       ACTIVE     OBJ_SZ       OBJS      WASTE
  dentry                       237.7 MiB   189.9 MiB    192.0 B     98765   47.8 MiB
```

`WASTE` = allocated-but-unfilled slab space (internal fragmentation).
Persistently huge dentry/inode slabs are normal on busy filesystems.

### `zram`

Swap totals plus per-device zram telemetry (disksize, original vs compressed
size, ratio, same/huge pages). `--sample SECS` adds pswpin/out rates over
that window from `/proc/vmstat`.

```sh
memreduct zram --sample 10
```

### `reclaim`

Targeted cgroup v2 reclaim via `memory.reclaim`: squeeze a subtree instead
of dropping global caches. Requires cgroup v2 + root + the memory controller
delegated to that cgroup.

```sh
sudo memreduct reclaim --cgroup user.slice/user-1000.slice/app-org.x.Browser-deadbeef.scope
sudo memreclaim --bytes 512M --cgroup my.slice        # best-effort amount
memreduct reclaim --dry-run
```

Without `--bytes` (or `--bytes max`/`0`) the kernel does best-effort reclaim
of whatever it can. Find candidate scopes with
`systemctl list-units --state=running` or `systemd-cgls`.

### `limit show` / `limit set`

Inspect or set `memory.high` (throttle point) and `memory.max` (hard cap +
OOM territory) on any cgroup v2 path.

```sh
memreduct limit show                                    # current cgroup
sudo memreduct limit show user.slice/user-1000.slice
sudo memreduct limit set app-org.x.Browser-deadbeef.scope high 2G
sudo memreduct limit set app-org.x.Browser-deadbeef.scope max 4G
sudo memreduct limit set app-org.x.Browser-deadbeef.scope max max      # unlimited
```

`limit show` also surfaces `memory.events` counters (low/high/max/oom/
oom_kill/oom_group_kill) — nonzero `high` counts mean the workload keeps
hitting its throttle.

## Recipes

### Tame a leaky Electron app without root

If your session delegates cgroups (default on most systemd desktops):

```sh
# find the running scope name for the app
systemctl --user list-units | grep app-
# cap it (memory.high throttles allocation before OOM)
memreduct limit set <scope-name> high 3G
```

### Watchdog pipeline with jq

```sh
while true; do
  memreduct --json status \
    | jq -r 'select(.used_percent >= 90) | "\(.used_bytes) used"'
  sleep 60
done
```

Every command emits exactly one JSON document per invocation; keys are
stable snake_case and safe to script against.

## Kernel/distro notes

- **PSI** (`psi`, `watch --psi`, pressure fields): kernel ≥ 4.20, needs
  `CONFIG_PSI=y`. Some vendor kernels ship PSI compiled but disabled — check
  `/proc/pressure/memory` exists. Only ONE trigger may own each pressure
  file system-wide (EBUSY otherwise).
- **smaps_rollup** (`pss`, `grow`): kernel ≥ 4.14.
- **slabinfo**: root-only readable on kernels ≥ ~3.x hardened configs
  (Debian/Fedora/Arch all restrict it).
- **cgroup v2**: `reclaim`/`limit` need the unified hierarchy (every major
  distro since ~2021). On legacy cgroup v1 systems these commands fail with
  an explanatory error.
- **drop_caches / compact_memory**: always root-writable only.
- **zram**: present by default on Fedora, ChromeOS, many ARM/embedded images;
  opt-in elsewhere (`zram-generator`).

## Troubleshooting

| Symptom | Cause & fix |
| --- | --- |
| `Permission denied (os error 13)` writing drop_caches | Not root: prefix `sudo` |
| PSI open fails with hint about write access | Triggers need root: `sudo memreduct watch --psi` |
| `EBUSY` registering PSI trigger | Another watcher owns the pressure file (another memreduct, `pressure` daemons, some monitors) |
| `EINVAL` registering PSI trigger | Window must be 500ms–10s: keep `--interval` within 1–10 when using `--psi` |
| `memory.reclaim missing` | Memory controller not delegated to that cgroup; check `cat /sys/fs/cgroup/cgroup.subtree_control` |
| `cgroup v2 unified hierarchy not found` | Legacy cgroup v1 host; reclaim/limit unsupported there |
| `read /proc/slabinfo ... Permission denied` | Run as root |
| `--swap-threshold applies only to polling mode` | Drop `--psi` or drop the swap threshold |

## Exit codes

`0` success; `1` any error (with a context chain on stderr explaining which
file failed and how to fix it). `watch` loops keep running on transient read
errors unless `--once`.
