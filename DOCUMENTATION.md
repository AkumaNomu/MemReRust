# MemReRust — Personal Documentation

> Your single-file handbook for `memreduct` v0.2.0. Keep this open while you work. Every example is copy-pasteable and tested against the actual binary.

---

## What this tool does

`memreduct` is your Linux port of Henry++ Mem Reduct. It gives you one static binary that can:

- snapshot RAM, swap, cache and PSI pressure
- drop clean caches or compact fragmented memory (root)
- watch memory/swap pressure and auto-clean
- rank processes by real memory cost (PSS), OOM kill priority, or leak rate
- inspect kernel slab caches and zram compression
- throttle or cap any cgroup v2 app via `memory.high` / `memory.max`

No daemon, no config file. Everything is a subcommand, every subcommand has `--json` so you can pipe it to `jq`, a script, or systemd.

---

## Install it for yourself

### Option 1: Use a release binary (any distro, no build tools needed)

Grab the tarball for your machine from [Releases](https://github.com/AkumaNomu/MemReRust/releases):

```sh
# x86_64 desktops/servers
curl -LO https://github.com/AkumaNomu/MemReRust/releases/latest/download/memreduct-v0.2.0-x86_64-unknown-linux-musl.tar.gz
tar xzf memreduct-v0.2.0-x86_64-unknown-linux-musl.tar.gz
sudo install -Dm755 memreduct /usr/local/bin/memreduct

# aarch64 (Pi, ARM servers)
# memreduct-v0.2.0-aarch64-unknown-linux-musl.tar.gz
```

Verify:

```sh
memreduct --version
memreduct status
```

### Option 2: Build from source

```sh
git clone https://github.com/AkumaNomu/MemReRust
cd MemReRust
cargo build --release
sudo install -Dm755 target/release/memreduct /usr/local/bin/memreduct
```

Build deps by distro:

| Your distro | Install |
|-------------|---------|
| Debian / Ubuntu | `sudo apt install build-essential cargo` |
| Fedora | `sudo dnf group install development-tools && sudo dnf install rust cargo` |
| Arch | `sudo pacman -S base-devel rust` |
| Alpine | `sudo apk add cargo rust` |
| openSUSE | `sudo zypper install rust cargo gcc` |

Fully static musl build (works even on stripped-down systems) — the recipe that actually works in the Flatpak SDK and in CI:

```sh
SYSROOT=$(rustc --print sysroot)
RUSTFLAGS="-C linker-flavor=ld.lld -C linker=$SYSROOT/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld" \
  cargo build --release --target x86_64-unknown-linux-musl
```

### Enable shell completions

```sh
# bash
memreduct completions bash | sudo tee /usr/share/bash-completion/completions/memreduct >/dev/null

# zsh
memreduct completions zsh | sudo tee /usr/share/zsh/site-functions/_memreduct >/dev/null

# fish
memreduct completions fish | sudo tee /usr/share/fish/vendor_completions.d/memreduct.fish >/dev/null
```

Restart your shell after.

---

## 30-second quick start

```sh
memreduct status                          # see where you are
memreduct pss --top 10                    # who is actually eating RAM
sudo memreduct slab --top 10              # kernel side
memreduct oom --top 10                    # who the kernel would kill first
memreduct zram                            # swap / zram health

# one-off leak check (wait 60s, show anything that grew by 32M+)
sudo memreduct grow --interval 60 --min 32M

# dry-run what watch would do
memreduct watch --once --threshold 90 --dry-run
```

---

## Every command, with when to use it

### Check overall health with `status`

Shows `MemTotal`, `MemAvailable`, `MemFree`, `Cached`, `Buffers`, `SReclaimable`, swap, and PSI. This is what you run first when the machine feels slow.

```sh
memreduct status
memreduct --json status | jq .
```

Output:

```
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

- `used = total - available` — the kernel's own definition, not `total - free`.
- `pressure` needs kernel 4.20+ with `CONFIG_PSI=y` and `/proc/pressure/memory` present. If it says `unavailable`, PSI is disabled or your kernel is too old.
- Needs no root.

JSON keys: `total_bytes`, `used_bytes`, `free_bytes`, `available_bytes`, `cached_bytes`, `buffers_bytes`, `reclaimable_bytes`, `used_percent`, `swap_total_bytes`, `swap_used_bytes`, `psi_available`, `psi_some_avg10_pct` etc.

### Flush caches with `clean`

Writes to `/proc/sys/vm/drop_caches` after a `sync()`. You almost never need this day-to-day — Linux reclaims caches on its own, and dropping them hurts I/O. Use it for benchmarks or to test OOM paths.

```sh
sudo memreduct clean                      # all (value 3)
sudo memreduct clean --mode page-cache    # value 1
sudo memreduct clean --mode slab          # value 2
memreduct clean --dry-run                 # preview, needs no root
memreduct --json clean --dry-run | jq .
```

Requires root. JSON returns `reclaimed_bytes` as the drop in `cached+buffers+reclaimable`.

### Defragment memory with `compact`

Writes `1` to `/proc/sys/vm/compact_memory`. It doesn't free memory — it shuffles pages to make large contiguous blocks, which helps big allocations.

```sh
sudo memreduct compact
sudo memreduct compact --dry-run
memreduct --json compact --dry-run | jq .
```

The write blocks until compaction finishes (can be a few seconds on big RAM). The before/after `MemFree` delta is informational. Needs root + `CONFIG_COMPACTION` (stock on all major distros).

### Watch and auto-clean with `watch`

Your automatic babysitter. Two engines:

- **Polling** (default): wake up every `--interval` seconds, check `used%`.
- **PSI** (`--psi some` or `--psi full`): register a kernel trigger on `/proc/pressure/memory`, sleep in `poll()` until pressure crosses the threshold. Near-zero CPU. Needs root and kernel 4.20+.

Common flags:

| Flag | What it does |
|------|--------------|
| `--threshold 90` | `used%` (or `avg10` with `--psi`) at which to clean |
| `--interval 30` | seconds between checks (polling) or PSI window (1–10 clamped) |
| `--mode all` | which caches to drop: `page-cache`, `slab`, `all` |
| `--cooldown 300` | don't clean again for 300s after a clean — stops thrash loops |
| `--exec 'notify-send ...'` | run a shell command after each successful clean |
| `--swap-threshold 70` | also clean when swap used% hits 70 (polling only) |
| `--once` | check once and exit — perfect for cron/systemd timers |
| `--dry-run` | print what would happen, don't touch caches |

Examples you can steal:

```sh
# Desktop: poll every 15s, also watch swap, notify on clean
sudo memreduct watch --threshold 92 --interval 15 --swap-threshold 70 \
  --cooldown 300 --exec 'notify-send -u critical "memreduct cleaned $(date +%T)"'

# Server: event-driven, no polling, cheaper than a loop
sudo memreduct watch --psi some --threshold 8 --mode page-cache

# Cron/timer friendly
memreduct watch --once --threshold 90 --dry-run

# Quiet JSON for logging
sudo memreduct --json watch --once --threshold 90 --dry-run | jq .
```

During cooldown, polling mode prints `memory 61% / swap 55%; thresholds 90/70% (cooling down)`. PSI mode prints `pressure some 12.3%; threshold 8% (cooling down)`. JSON mode stays silent unless it actually cleans.

`--swap-threshold` only works with polling. If you pass it with `--psi`, you get an error immediately instead of being silently ignored.

### Systemd: keep `watch` alive without babysitting it

Two patterns are in `packaging/systemd/`:

**Resident service** — `watch` stays running:

```sh
sudo cp packaging/systemd/memreduct-watch.service.example /etc/systemd/system/memreduct-watch.service
sudo systemctl daemon-reload
sudo systemctl enable --now memreduct-watch.service
```

**Timer** — zero resident memory, checks every 5 min:

```sh
sudo cp packaging/systemd/memreduct-clean.service.example /etc/systemd/system/memreduct-clean.service
sudo cp packaging/systemd/memreduct-clean.timer.example /etc/systemd/system/memreduct-clean.timer
sudo systemctl daemon-reload
sudo systemctl enable --now memreduct-clean.timer
```

The service file is hardened (`NoNewPrivileges`, `ProtectSystem=strict`, `CAP_SYS_ADMIN` only, etc.).

### Rank processes honestly with `pss`

`ps` and `top` show RSS, which double-counts shared pages. PSS splits them fairly.

```sh
memreduct pss                       # top 10 among processes you can see
sudo memreduct pss --top 20         # whole system (needs root for others' smaps)
memreduct pss 1234 5678             # just these PIDs
memreduct --json pss --top 5 | jq .
```

Needs kernel 4.14+ (`smaps_rollup`). Without root you only see your own processes — the output says `no accessible processes` if nothing is readable.

Columns: `RSS`, `PSS`, `PSS_Anon`, `PSS_File`, `SWAP` internally; table shows `RSS` / `PSS` / `SWAP`. JSON has all five `*_bytes` fields.

### Catch leaks with `grow`

Takes two sweeps `--interval` seconds apart, matches processes by PID + starttime (so PID reuse can't fake growth), reports anything that grew by `--min`.

```sh
# wait 2 minutes, flag anything that grew by 16MiB or more
sudo memreduct grow --interval 120 --min 16M
memreduct --json grow --interval 30 --min 32M | jq .

# watch a specific growth rate
sudo memreduct grow --interval 60 --min 10M
```

Output includes delta and rate (`+64.2 MiB (1.0 MiB /s)`).

### Find who the OOM killer will pick with `oom`

Cheap — reads `statm` and `oom_score`/`oom_score_adj`, sorted worst first. Also shows total `oom_kill` events since boot.

```sh
memreduct oom --top 10
sudo memreduct oom --top 20        # system-wide
memreduct --json oom --top 5 | jq .
```

`SCORE` 0–1000, `ADJ` -1000..1000. `-1000` means unkillable (systemd, kernel threads). If you see `kernel OOM kills since boot: 3 (!)`, something already got killed.

### Inspect kernel slab caches with `slab`

Where the kernel hides memory: dentries, inodes, kmalloc buckets, conntrack...

```sh
sudo memreduct slab --top 10
sudo memreduct --json slab --top 5 | jq .
```

Output:

```
slab total 1.2 GiB (top 5 of 93 caches)
  CACHE                            SIZE       ACTIVE     OBJ_SZ       OBJS      WASTE
  dentry                       237.7 MiB   189.9 MiB    192.0 B     98765   47.8 MiB
```

`WASTE` is slab space allocated but not holding a live object. Big `dentry`/`inode_cache` is normal on busy filesystems.

Needs root on every mainstream distro. If you see `read /proc/slabinfo ... Permission denied`, add `sudo`.

### Check swap and zram with `zram`

```sh
memreduct zram
memreduct zram --sample 10         # measure swap thrash for 10s
memreduct --json zram | jq .
```

Per-device: `disksize`, `orig` vs `compressed` size, `ratio`, `mem_used`, `same_pages`, `huge_pages`. With `--sample`, adds `pswpin`/`pswpout` pages/sec from `/proc/vmstat`.

Zram is present by default on Fedora, ChromeOS, many ARM images. On Debian/Ubuntu/Arch, enable it with `zram-generator`.

### Target a cgroup instead of the whole machine with `reclaim`

Squeeze one cgroup tree instead of dropping global caches. Needs cgroup v2 + root + memory controller delegated to that cgroup.

```sh
# find candidates
systemctl --user list-units | grep app-
systemd-cgls

# best-effort reclaim of a browser scope
sudo memreduct reclaim --cgroup user.slice/user-1000.slice/app-org.x.Browser-deadbeef.scope

# try to free ~512M
sudo memreduct reclaim --bytes 512M --cgroup my.slice

# preview
memreduct reclaim --cgroup my.slice --dry-run
memreduct --json reclaim --cgroup my.slice --dry-run | jq .
```

Without `--bytes` (or with `0`/`max`) the kernel does best-effort. Error `memory.reclaim missing` means the memory controller isn't delegated — check `cat /sys/fs/cgroup/cgroup.subtree_control` and delegate via systemd `Delegate=yes` or `systemctl set-property ... Delegate=memory`.

Without `--cgroup`, it targets your current cgroup.

### Cap a cgroup with `limit`

`memory.high` = throttle point (kernel slows the cgroup), `memory.max` = hard cap (allocations fail / OOM inside the cgroup).

```sh
memreduct limit show
sudo memreduct limit show user.slice/user-1000.slice/app-firefox.slice
sudo memreduct limit set app-org.x.Browser.scope high 2G
sudo memreduct limit set app-org.x.Browser.scope max 4G
sudo memreduct limit set app-org.x.Browser.scope max max   # remove cap
memreduct --json limit show | jq .
```

`limit show` also prints `memory.events` counters (`low/high/max/oom/oom_kill/oom_group_kill`). If `high` keeps climbing, the app lives throttled.

### Read pressure stalls with `psi`

```sh
memreduct psi
sudo memreduct psi --cgroup user.slice/app-firefox.scope
memreduct --json psi | jq .
```

`some` = at least one task stalled, `full` = all tasks stalled. `avg10/60/300` are percentages, `total` is microseconds. Needs PSI as with `watch --psi`. Without PSI you get `unavailable`.

### Shell completions with `completions`

```sh
memreduct completions bash   # pipe to your completion dir, see Install
memreduct completions zsh
memreduct completions fish
```

Piping to `head` or `grep -q` exits quietly — the binary handles `SIGPIPE` like a well-behaved Unix tool.

---

## Use `--json` for scripting

Every command accepts `--json` and emits **one JSON document per invocation** on stdout, snake_case keys, stable across v0.2.x. Errors go to stderr with a context chain naming the file and fix.

```sh
# alert when used% >= 90
memreduct --json status | jq -e 'select(.used_percent >= 90) | halt_error(0)' \
  && notify-send "RAM high"

# find the worst offender
memreduct --json pss --top 5 | jq -r '.[0] | "\(.pid) \(.comm) \(.pss_bytes)"'

# log compact deltas
sudo memreduct --json compact | jq '{free_before_bytes, free_after_bytes}'

# cheapest health check for a dashboard
memreduct --json status | jq '{used_percent, swap_used_percent: ((.swap_used_bytes*100)/.swap_total_bytes | floor)}'
```

All sizes in JSON are `u64` bytes. Parse user-supplied sizes with `K/M/G/T` suffixes (case-insensitive, `KiB`/`MiB` also work): `512`, `1K`, `2M`, `3G`, `1T`.

---

## Your real-world recipes

### Keep a leaky Electron app under control without sudo

If your desktop delegates cgroups (default on systemd desktops):

```sh
systemctl --user list-units | grep app-
memreduct limit set app-firefox.scope high 3G   # no sudo needed if delegated
```

`memory.high` throttles before the machine OOMs — often enough.

### Desktop watchdog that notifies you

```sh
sudo memreduct watch --threshold 90 --interval 20 --swap-threshold 70 \
  --cooldown 300 --exec 'notify-send -u critical "memreduct cleaned $(date)"'
```

### Server that reacts to PSI stalls

```sh
sudo memreduct watch --psi full --threshold 5 --mode page-cache --cooldown 600
```

### Nightly leak scan via cron

```sh
0 3 * * * /usr/local/bin/memreduct --json grow --interval 120 --min 64M | /usr/bin/jq -e '.growers | length > 0' && /usr/bin/mail -s "leak?" you@example.com
```

### One-liner health snapshot for support tickets

```sh
memreduct status; echo "---"; sudo memreduct slab --top 5; echo "---"; memreduct zram
```

---

## Build it, package it, run it in CI

The binary has no runtime config and only four deps (`anyhow`, `clap`, `clap_complete`, `libc`). Tests are parser-only and need no root.

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
```

Flatpak SDK / CI note: that environment has no system `cc`. Provide a shim:

```sh
export PATH=/tmp/opencode/tools/bin:$PATH   # cc -> zig cc
```

Static builds for every distro are plain musl (`packaging/` has Arch/Alpine/Fedora/Debian recipes and hardened systemd units under `packaging/systemd/`):

```sh
SYSROOT=$(rustc --print sysroot)
RUSTFLAGS="-C linker-flavor=ld.lld -C linker=$SYSROOT/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld" \
  cargo build --release --target x86_64-unknown-linux-musl
```

CI builds both `x86_64` and `aarch64` musl via `cargo-zigbuild` and attaches tarballs to tags.

With the `gui` feature:

```sh
cargo build --features gui --release
./target/release/memreduct gui          # or --gui, or no args
cargo build --features gui --release --target x86_64-unknown-linux-musl  # static GUI still needs display libs at runtime
```

Windows GUI build (PowerShell):

```powershell
cargo build --features gui --release
.\target\release\memreduct.exe gui
```

---

## Use the compact GUI (Windows + Linux)

The new GUI is a single native window (egui, no browser/Electron), ~15MB, dark by default, built with `--features gui`. It exposes every CLI capability alongside live autoclean — compact but complete.

### Launch it

```sh
memreduct gui          # explicit subcommand
memreduct --gui        # flag — same
memreduct              # no subcommand → GUI if you built with --features gui, otherwise help
```

On Windows the same three forms work (`memreduct.exe gui`).

### What you see

- **Top bar** — `Mem Reduct v0.2.0`, platform badge, **↻ Refresh**.
- **Memory cards** — RAM and swap as colored progress bars (green <70%, amber <90%, red ≥90%), free/avail, cached/reclaimable, live PSI `some`/`full` when available. Same numbers as `status`.
- **History strip** — `used %` over the last ~2.5 min (300 points, 500ms poll). No extra deps.
- **Quick actions** — `Clean all` / `PageCache` / `Slab` / `Compact` + `☐ Auto-clean` toggle. These call the same `drop_caches` / `compact_memory` paths as the CLI, so they need root (Linux) / admin (Windows) and show the error in the bottom status bar instead of panicking.
- **Status bar** — last message + `● auto-clean on` indicator + current `used%`.

### Tabs (one window, everything)

| Tab | What it does | CLI equivalent |
|-----|--------------|---------------|
| **Overview** | Full `status` grid + collapsible **Auto-clean settings** (threshold, swap, interval, cooldown, mode, PSI toggle + metric, Exec command) | `status`, `watch` |
| **Processes** | Top-N by PSS, sortable, Refresh | `pss --top N` |
| **OOM Scores** | Top-N by `oom_score`, colored when >500, plus `oom_kill` count | `oom --top N` |
| **Slab** | Top-N caches by size, with waste | `slab --top N` |
| **Zram** | Per-device disksize/orig→comp/ratio/mem + swap totals | `zram` |
| **Cgroup** | `Reclaim`, `Show limits`, `Set high/max` for a path (Linux-only; Windows shows a no-op note) | `reclaim`, `limit show/set` |
| **Leak Scan** | Interval + Min inputs, **Start scan** (background thread, spinner, results table with delta) | `grow --interval --min` |

All tabs have **Top** / **Refresh** controls. Tables use `format_bytes` so you see `MiB`/`GiB`, not raw bytes.

### Auto-clean inside the GUI

Toggle `Auto-clean` in the quick-actions row, then open **Overview → Auto-clean settings** and set:

- `Threshold %` (RAM `used%`, or PSI `avg10` when `use PSI` is on)
- `Swap %` (0 = off, polling only — GUI enforces the same rule as CLI: PSI + swap errors)
- `Interval s` / `Cooldown s` / `Mode` / `Exec after clean`

The GUI reuses the exact `AutoClean` logic from `watch` (500ms poll, cooldown gate, `sh -c` exec). It runs on the UI thread but never blocks — the next poll is `request_repaint_after(500ms)`.

### Platform notes

- **Linux** — every tab works. `clean`/`compact`/`slab` need root; the GUI shows `need root` in the status bar if you run without `sudo`/`pkexec`.
- **Windows** — `status`/`processes`/`oom` work via `GlobalMemoryStatusEx` + `EmptyWorkingSet` (tries `PROCESS_SET_QUOTA` for the current process; needs admin for others). `PSI`/`slab`/`zram`/`cgroup` show “Not available on Windows” and are disabled. The same binary and same `memreduct gui` command work on both — no separate build.
- **Headless** — `memreduct gui` on a server with no display exits with `GUI not built — rebuild with --features gui (needs a display server)` (or a winit “no display” error). The CLI keeps working.

### Build the GUI where you are

```sh
# Debian/Ubuntu
sudo apt install libxkbcommon-dev libwayland-dev libxcb1-dev
cargo build --features gui --release

# Fedora
sudo dnf install wayland-devel libxkbcommon-devel
cargo build --features gui --release

# Arch
sudo pacman -S wayland libxkbcommon
cargo build --features gui --release

# Windows (no extra dev libs)
cargo build --features gui --release
```

The GUI feature is **optional** — `cargo test` / `cargo build` without it still builds the fast CLI and never pulls `eframe`/`egui_plot`/`winit`/`wayland`.

---

## What needs root and what doesn't

| Command | Root? | Why |
|---------|-------|-----|
| `status`, `psi`, `pss`, `grow`, `zram`, `oom`, `completions` | no | read-only `/proc` you can already see |
| `clean`, `compact` | yes | write to `/proc/sys/vm/*` |
| `watch --psi` (trigger registration) | yes | write to `/proc/pressure/memory` |
| `reclaim`, `limit` | yes + cgroup v2 delegation | write to `memory.*` under `/sys/fs/cgroup` |
| `slab` | yes on most distros | `/proc/slabinfo` is 0400 |

If you get `Permission denied`, prefix `sudo`. If PSI says `unavailable` or you get `cgroup v2 unified hierarchy not found`, your kernel/distro is too old or on legacy v1 cgroups — reclaim/limit and PSI features won't work there.

---

## Troubleshooting

| You see | Do this |
|---------|---------|
| `Permission denied (os error 13)` writing `drop_caches` / `compact_memory` | run with `sudo` |
| PSI open fails mentioning write access | triggers need root: `sudo memreduct watch --psi ...` |
| `EBUSY` registering PSI trigger | something else already owns that pressure file (another watcher) |
| `EINVAL` registering PSI trigger | window must be 500ms–10s — keep `--interval` 1–10 with `--psi` |
| `memory.reclaim missing` | memory controller not delegated: `cat /sys/fs/cgroup/cgroup.subtree_control` should list `memory` |
| `cgroup v2 unified hierarchy not found` | host is on cgroup v1 — reclaim/limit unsupported |
| `read /proc/slabinfo ... Permission denied` | use `sudo` |
| `--swap-threshold applies only to polling mode` | drop `--psi` or drop the flag |
| `no accessible processes` from `pss`/`oom` | you can only see your own procs — use `sudo` for system-wide |

Exit codes: `0` on success, `1` on any error. `watch` without `--once` keeps running through transient read errors.

---

## Where to look next

- `README.md` — project elevator pitch and install pointers
- `GUIDE.md` — shorter reference (this file is the full personal manual)
- `AGENTS.md` — build internals, source layout, env caveats for contributors
- `memory.md` — session log and backlog
- `packaging/` — installable recipes for Arch / Alpine / Fedora / Debian plus systemd examples
- `CHANGELOG.md` — what changed in v0.2.0

GPL-3.0-only. Upstream Windows original at <https://github.com/henrypp/memreduct>; this fork at <https://github.com/AkumaNomu/MemReRust>.
