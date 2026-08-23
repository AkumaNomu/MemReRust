# Mem Reduct for Linux

Rust CLI rewrite of Henry++ Mem Reduct, rebuilt for Linux.

## Build

```sh
cargo build --release
```

## Use

```sh
memreduct status
sudo memreduct clean
memreduct clean --mode page-cache --dry-run
memreduct watch --threshold 90 --interval 30
memreduct watch --psi some --threshold 5
memreduct psi
memreduct pss --top 20
memreduct grow --interval 60 --min 64M
memreduct zram
memreduct reclaim --cgroup user.slice/app-org.x.Browser-deadbeef.scope
memreduct limit set user.slice/app-org.x.Browser-deadbeef.scope max 2G
memreduct limit show
memreduct --json status
```

`watch --psi` registers a kernel PSI trigger (`<metric> <stall-us> <window-us>` written to `/proc/pressure/memory`) and blocks in `poll()` until pressure crosses the threshold, which consumes less CPU than a polling loop. Registering triggers requires write access to `/proc/pressure/memory` (run as root); the window is clamped to 1-10 seconds. Requires kernel 4.20+ with PSI exposed at `/proc/pressure`.

`reclaim` and `limit` operate on cgroup v2 memory (systemd scopes/slices). They require cgroup v2, root, and delegating the memory controller to your session (a container or a systemd user session normally sets this up for you). `pss` and `grow` use `smaps_rollup` (kernel 4.14+) for per-process PSS attribution and growth tracking; `zram` reports swap/zram telemetry.

`clean` runs `sync`, then writes Linux-native `drop_caches`. Root required.
Linux cannot safely expose Windows working-set, standby-list, or registry-cache operations through this interface.

Cache dropping is mainly for testing/debugging. Linux reclaims caches automatically; forced cleanup can hurt I/O performance.

GPL-3.0-only. Original project: <https://github.com/henrypp/memreduct>.
