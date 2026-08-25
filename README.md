# Mem Reduct for Linux

Rust CLI rewrite of [Henry++ Mem Reduct](https://github.com/henrypp/memreduct), rebuilt for Linux: monitor memory pressure, drop caches, trigger kernel compaction, watch PSI, inspect per-process PSS/OOM scores, and manage cgroup v2 memory — all from one dependency-light static binary with `--json` on every command.

```sh
memreduct status                 # RAM/swap/cache/PSI snapshot
sudo memreduct clean             # sync + drop_caches (page cache + slab)
sudo memreduct compact           # fight fragmentation via compact_memory
sudo memreduct slab --top 10     # biggest kernel slab caches
memreduct oom --top 5            # who gets killed first
memreduct pss --top 20           # honest per-process memory (smaps_rollup)
memreduct grow --interval 60     # ΔPSS leak detector
memreduct zram --sample 10       # swap/zram compression telemetry
memreduct --json status | jq .used_percent

# auto-clean when thresholds trip (polling or event-driven PSI triggers)
sudo memreduct watch --threshold 90 --cooldown 300 \
    --exec 'notify-send memreduct "cleaned $(date +%T)"'
sudo memreduct watch --psi some --threshold 8

# cgroup v2 targeted control
sudo memreduct reclaim --cgroup user.slice/app-org.x.Browser-deadbeef.scope
sudo memreduct limit set app-org.x.Browser-deadbeef.scope high 2G
```

## Install

- **Any distro**: fully static musl binaries (x86_64/aarch64) from [Releases](https://github.com/AkumaNomu/MemReRust/releases).
- **From source**: `cargo install --git https://github.com/AkumaNomu/MemReRust`
- **Packaging recipes** for Arch (`PKGBUILD`), Alpine (`APKBUILD`), Fedora/RHEL (`.spec`), Debian/Ubuntu (`debian/`), plus systemd unit examples: see [`packaging/`](packaging/).

Full documentation, recipes, troubleshooting, and kernel-version notes live in the [guide](GUIDE.md).

## Notes

- `watch --psi` registers a kernel PSI trigger on `/proc/pressure/memory` and blocks in `poll()` until pressure crosses the threshold — cheaper than polling. Needs root and kernel 4.20+.
- `pss`/`grow` use `smaps_rollup` (kernel 4.14+) for proportional set sizes; PID reuse is filtered by process starttime matching.
- `clean` runs `sync`, then writes `/proc/sys/vm/drop_caches`. Root required. Linux cannot safely expose Windows working-set, standby-list, or registry-cache operations through this interface.
- Cache dropping is mainly for testing/debugging; Linux reclaims caches automatically and forced drops can hurt I/O performance.

GPL-3.0-only. Original project: <https://github.com/henrypp/memreduct>.
