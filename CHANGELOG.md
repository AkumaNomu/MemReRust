# Changelog

All notable changes to the Linux rewrite of Mem Reduct.

The changelog history of the original Windows project is kept upstream:
<https://github.com/henrypp/memreduct/blob/master/changelog.txt>

## 0.1.0 - Unreleased

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
