# memory.md

Persistent working memory for this repo: environment facts, audit history,
and the prioritized backlog. Append a new dated entry per work session;
never delete prior sessions.

## Environment facts

- Dev box runs **Freedesktop SDK 25.08 Flatpak runtime** (x86_64): no system
  C toolchain, no sudo inside sandbox. See "Build environment caveat" in
  [AGENTS.md](AGENTS.md) for the zig-as-cc shim (`/tmp/opencode/tools/bin/cc`,
  tmpfs — recreate after reboot).
- gnu host target cannot link here (no crt1.o); **musl target works** via
  rustup's self-contained sysroot. Verify with:
  `PATH=/tmp/opencode/tools/bin:$PATH cargo test --target x86_64-unknown-linux-musl`
- Root-required paths (`drop_caches`, `compact_memory`, slabinfo, PSI trigger
  writes) are NOT exercisable in this sandbox; they're covered by parser unit
  tests + must be smoke-tested on a real machine before releases.
- Remote: origin = github.com/AkumaNomu/MemReRust (SSH), upstream = henrypp.
- Toolchain: rustc/cargo 1.97.1 stable.

## Session log

### 2026-08-25 — feature wave 1, packaging, docs overhaul

Baseline audit found: no linker (env), one pre-existing fmt violation
(`src/zram.rs` blank line), everything else green (12 tests, clippy clean).

Implemented:

1. `compact` subcommand — `/proc/sys/vm/compact_memory` write, dry-run;
   reporting semantics corrected during re-audit (compaction relocates
   pages, does not free; MemFree delta is informational only).
2. `slab` subcommand — new `src/slab.rs`; canonical `" : tunables"` /
   `" : slabdata"` split parsing; derives slab counts when section missing;
   4 unit tests incl. malformed-line tolerance.
3. `oom` subcommand — `procs::read_oom_score`, cheap `statm` RSS reader,
   `/proc/vmstat` oom_kill counter; sorted by score.
4. `watch` hardening — `AutoClean` struct (cooldown rate limit + post-clean
   `--exec sh -c` hook); `--swap-threshold PCT` polling trigger; PSI mode
   rejects swap-threshold upfront instead of ignoring; default cooldown 300s.
5. `completions bash|zsh|fish` via clap_complete (new dep, version 0.2.0).
6. Fixes: `MemFree` parsed into `Memory` (+status human/JSON);
   `parse_size` T suffix; cgroup v1 error messages name the actual problem.
7. Packaging: `packaging/{arch/alpine/fedora/debian/systemd}` + GitHub
   workflows `ci.yml` (fmt/clippy/test/musl-static matrix) and `release.yml`
   (cargo-zigbuild static binaries attached to tags).
8. Docs: rewrote README, wrote full GUIDE.md, updated CHANGELOG (0.2.0),
   created AGENTS.md and this file.

Verification at session end: fmt clean, 18 tests pass, clippy -D warnings
exit 0, host debug build OK, static-pie musl release build OK and runs.
Live smoke: `status`, `watch --once` (memory+swap thresholds, cooling-down
path), `oom`, `pss`, `completions bash/zsh/fish`, `--json` variants parse
with `python3 -m json.tool`. `--top 0` rejected via clap range(1..) on
pss/slab/oom. slab JSON validated by unit tests only (needs root).

Key discovery (musl linking): zig cc CANNOT link the musl target here — it
injects its own crt1.o alongside rustc's self-contained rcrt1.o → duplicate
`_start`. Working recipe links rustup's bundled lld directly (see AGENTS.md
"Commands"): `-C linker-flavor=ld.lld -C linker=$SYSROOT/.../bin/rust-lld`.
CI avoids all of this via cargo-zigbuild.

State at session end: ALL CHANGES UNCOMMITTED on upstream-master working
tree (user has not asked for commits). `git add -A && git commit` when told.

### 2026-08-25 (later) — re-audit of session 1

Adversarial pass over all session-1 code. Findings + fixes:

1. **BrokenPipe panic** (real bug): `memreduct completions <shell> | head`
   panicked inside clap_complete's writer because Rust ignores SIGPIPE.
   Fixed by restoring SIG_DFL for SIGPIPE at startup (`libc::signal`) —
   canonical CLI behavior, verified silent rc=141 death. Affects every
   command's piped usage, not just completions.
2. **compact reporting semantics wrong** (see session 1 item 1): dropped
   misleading `freed_bytes` JSON key; GUIDE section rewritten (the write
   blocks until kernel compaction finishes — not async as first written).
3. **packaging/debian/compat removed**: having both the compat file and
   `debhelper-compat (= 13)` in control is a debhelper error.
4. **PSI/poll cooldown parity**: PSI mode was silent during cooldown while
   poll mode printed "(cooling down)"; both print it in human mode now.
5. **slab parser hardened**: split on canonical " : tunables" separator so
   cache names containing ':' parse correctly (+ test); version-line and
   empty-input cases covered by tests.
6. **slab() needless Vec clone removed**; cache_count captured pre-sort.
7. Tooling gotcha: clippy can exit 101 spuriously from stale incremental
   cache after out-of-band (sed/python) file edits — `cargo clean -p
   memreduct` or touch sources before trusting results.

Gates after fixes: fmt clean, 19 tests, clippy -D warnings exit 0, host +
static musl release builds OK, JSON contract re-verified for every
subcommand via python json.tool.

## Audit findings ledger

| Finding | Status |
| --- | --- |
| zram.rs stray blank line broke `cargo fmt --check` | fixed 2026-08-25 |
| No usable linker in flatpak runtime (gnu target dead) | worked around via zig cc shim; documented in AGENTS.md |
| PSI window clamp vs kernel EINVAL (500ms–10s) | verified correct: `interval.clamp(1,10)` |
| `watch_psi` ignored swap thresholds silently | fixed: explicit bail with hint |
| Auto-clean could thrash (no rate limit) | fixed: `--cooldown` default 300s |
| MemFree never surfaced despite being useful for compact deltas | fixed: Memory.free + status output |
| parse_size capped at G | fixed: T suffix + test |
| cgroup v1 systems got cryptic errors | fixed: explicit messages |
| JSON contract: every command emits exactly one document | re-verified all commands 2026-08-25 |
| BrokenPipe panic when piping into `head` (Rust ignores SIGPIPE) | fixed 2026-08-25 re-audit: SIG_DFL at startup |
| compact reported `freed_bytes` but compaction relocates, not frees | fixed 2026-08-25 re-audit: informational before/after only |
| debian/compat + debhelper-compat in control conflict | fixed 2026-08-25 re-audit: compat file removed |
| PSI cooldown suppression silent vs poll's "(cooling down)" | fixed 2026-08-25 re-audit: parity |
| slab names containing ':' truncated parser input | fixed 2026-08-25 re-audit: " : tunables" split + test |

## Backlog (prioritized)

1. **Root smoke tests on real hardware**: `clean`, `compact`, `slab`,
   `watch --psi` trigger registration, `reclaim`/`limit` under systemd user
   delegation. Blocked until access to a privileged box; see item 11.
2. **man page**: add `clap_mangen` behind an optional `--generate-man`
   hidden flag or build.rs step so distro packagers get a man page without
   hand-writing roff.
3. **`watch` JSON event stream**: emit structured tick events in JSON mode
   (currently silent unless cleaning) — useful for log-based monitoring;
   keep keys stable, gate behind explicit flag to avoid breaking pipelines.
4. **cgroup tree discovery**: helper that maps PID → owning scope/slice so
   `reclaim --pid N` / `limit set --pid N` work without users hunting scope
   names (`/proc/PID/cgroup` + mount join).
5. **PSI multi-window reporting**: expose avg60/avg300 in watch decisions or
   add `psi --follow` streaming mode.
6. **zram writeback stats**: parse `wb_stat` when kernel exposes it.
7. **aarch64 local verification**: try `rustup target add
   aarch64-unknown-linux-musl` + rust-lld `-m elf_aarch64` emulation locally;
   CI already covers it via cargo-zigbuild.
8. **shellcheck the packaging scripts** once shellcheck is available in env.
9. Consider `panic = "abort"` + LTO for release profile size (measure first;
   PKGBUILD sets `!lto` pending measurement).
10. **Compaction effectiveness metric**: diff `/proc/pagetypeinfo` free-page
    counts (per order/migratetype) before/after compact — the honest way to
    show fragmentation improvement instead of MemFree noise.
11. **Root smoke-test script** (`scripts/smoke-root.sh`) covering clean/
    compact/slab/watch --psi/reclaim/limit with pass/fail output; pairs
    with backlog item 1.
