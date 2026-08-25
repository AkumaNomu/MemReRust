# Changelog

All notable changes to the Linux rewrite of Mem Reduct.

The changelog history of the original Windows project is kept upstream:
<https://github.com/henrypp/memreduct/blob/master/changelog.txt>

## 0.2.0 - Unreleased

- Restore default SIGPIPE handling so piping into `head`/`grep -q` exits
  quietly instead of panicking in the writer
- `compact`: trigger kernel memory compaction via `/proc/sys/vm/compact_memory`
  (fragmentation relief; root + CONFIG_COMPACTION), with `--dry-run`; reports
  MemFree before/after as informational (compaction relocates pages rather
  than freeing them)
- `slab`: top slab cache consumers from `/proc/slabinfo` (size/active/waste
  per cache; root on most kernels)
- `oom`: rank processes by OOM killer score with `oom_score_adj`, statm RSS,
  and system-wide `oom_kill` event count
- `completions bash|zsh|fish`: shell completion scripts (clap_complete)
- `watch --cooldown SECS` (default 300): anti-thrash rate limit for
  automatic cleans, applies to both polling and PSI engines
- `watch --exec CMD`: run a shell command after each successful triggered
  clean (e.g. desktop notifications)
- `watch --swap-threshold PCT`: additional swap-based trigger (polling mode;
  explicitly rejected in PSI mode instead of being silently ignored)
- `status`: report `MemFree` alongside used/available (human + JSON)
- `parse_size` accepts T/TiB suffixes (TB-scale cgroup limits)
- Clearer errors when running on legacy cgroup v1 systems
- Packaging: Arch PKGBUILD, Alpine APKBUILD, Fedora RPM spec, Debian dir,
  systemd unit/timer examples (`packaging/`)
- CI: fmt/clippy/test gate plus static musl builds for x86_64 and aarch64;
  release workflow attaches binaries to tags

## 0.1.0 - 2026-08-16

- Ported Mem Reduct to Linux as a Rust CLI
- `status`: report physical, swap, and reclaimable-cache usage
- `clean`: drop page cache, slab, or all (`sync` + `drop_caches`), with `--dry-run`
- `watch`: poll usage and auto-clean when a threshold is reached
- `watch --psi`, `psi`: PSI (pressure stall information) event-driven monitoring via registered kernel PSI triggers with `some`/`full` metrics and stall-time thresholds, replacing polling for CPU-cheap watching (requires root)
- `pss`: per-process proportional set size from `smaps_rollup`
- `grow`: per-process ΔPSS leakage detection with rate reporting
- `zram`: swap and zram telemetry (incl. `mem_used_total`, `compr_data_size`, compression ratio, 4KiB-page I/O rates)
- `reclaim`: targeted low-level pressure-driven cgroup v2 reclaim via `memory.reclaim`
- `limit set|show`: cgroup v2 `memory.max`/`memory.high` control
- `--json` output for all commands
