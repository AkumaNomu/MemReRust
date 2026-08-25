# Mem Reduct for Linux

Rust CLI rewrite of [Henry++ Mem Reduct](https://github.com/henrypp/memreduct), rebuilt for Linux **and Windows**: monitor memory pressure, drop caches, trigger kernel compaction, watch PSI, inspect per-process PSS/OOM scores, and manage cgroup v2 memory — now with a compact native GUI for both platforms. The CLI is a single dependency-light static binary with `--json` on every command; the GUI is an optional `gui` feature (egui, one binary, no web runtime).

```sh
# CLI — same on Linux and Windows
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

# cgroup v2 targeted control (Linux only)
sudo memreduct reclaim --cgroup user.slice/app-org.x.Browser-deadbeef.scope
sudo memreduct limit set app-org.x.Browser-deadbeef.scope high 2G

# GUI — compact, native, cross-platform
memreduct gui                    # explicit
memreduct --gui                  # flag, same
memreduct                        # no args → GUI if built with --features gui
```

## Install

- **Any distro**: fully static musl binaries (x86_64/aarch64) from [Releases](https://github.com/AkumaNomu/MemReRust/releases).
- **From source (CLI)**: `cargo install --git https://github.com/AkumaNomu/MemReRust`
- **From source (with GUI)**: `cargo install --features gui --git https://github.com/AkumaNomu/MemReRust`
  - Linux needs Wayland/X11 dev libs (e.g. `libxkbcommon-dev` on Debian, `libxcb` on Arch); Windows needs no extra deps.
  - Build: `cargo build --features gui --release` then `./target/release/memreduct gui`
- **Packaging recipes** for Arch (`PKGBUILD`), Alpine (`APKBUILD`), Fedora/RHEL (`.spec`), Debian/Ubuntu (`debian/`), plus systemd unit examples: see [`packaging/`](packaging/).

**Your docs:** [DOCUMENTATION.md](DOCUMENTATION.md) is the full personal handbook (copy-paste examples, JSON recipes, systemd setup, cgroup deep-dive). [GUIDE.md](GUIDE.md) is the shorter reference, [docs/CHEATSHEET.md](docs/CHEATSHEET.md) is the one-pager to keep on your desk.

## Notes

- `watch --psi` registers a kernel PSI trigger on `/proc/pressure/memory` and blocks in `poll()` until pressure crosses the threshold — cheaper than polling. Needs root and kernel 4.20+.
- `pss`/`grow` use `smaps_rollup` (kernel 4.14+) for proportional set sizes; PID reuse is filtered by process starttime matching.
- `clean` runs `sync`, then writes `/proc/sys/vm/drop_caches`. Root required. Linux cannot safely expose Windows working-set, standby-list, or registry-cache operations through this interface.
- Cache dropping is mainly for testing/debugging; Linux reclaims caches automatically and forced drops can hurt I/O performance.

GPL-3.0-only. Original project: <https://github.com/henrypp/memreduct>.
