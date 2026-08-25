# AGENTS.md

Guidance for AI agents (and humans) working on this repository.

## Project

`memreduct` is a Rust CLI + optional native GUI that ports Henry++ Mem Reduct
to Linux and Windows. It reads `/proc`/sysfs on Linux (and `GlobalMemoryStatusEx`
/ `EmptyWorkingSet` on Windows), drives `drop_caches`, PSI triggers, cgroup-v2
memory controls, and zram telemetry. No daemon, no config files; everything is
subcommands + flags with optional `--json` on every command, plus `gui` / `--gui`
for the compact egui window.

Upstream (Windows original): <https://github.com/henrypp/memreduct>
This fork: <https://github.com/AkumaNomu/MemReRust>

## Source layout

| File | Responsibility |
| --- | --- |
| `src/main.rs` | CLI definition (`clap` derive), all command implementations, JSON output, `gui` dispatch |
| `src/gui.rs` | egui native GUI (feature `gui`, Windows + Linux, compact minimalist) |
| `src/mem.rs` | `/proc/meminfo` parsing, byte formatting/parsing, `drop_caches` path |
| `src/cgroup.rs` | cgroup v2 discovery via mountinfo, `memory.*` file IO |
| `src/psi.rs` | PSI parsing + kernel trigger registration (`poll(2)` based) |
| `src/procs.rs` | per-process readers: comm, stat starttime, smaps_rollup, statm, oom_score |
| `src/slab.rs` | `/proc/slabinfo` parsing and per-cache size math |
| `src/zram.rs` | zram device attrs, mm_stat, vmstat swap rates, page size |

## Build environment caveat (IMPORTANT)

This workspace is a Freedesktop SDK 25.08 Flatpak runtime with **no system C
toolchain** (`cc` absent, no crt1.o for the gnu target). The stock gnu target
cannot link here. Working setup:

```sh
export PATH=/tmp/opencode/tools/bin:$PATH   # provides `cc` -> zig cc shim
```

If `/tmp/opencode/zig` is gone (tmpfs reset), recreate it:

```sh
mkdir -p /tmp/opencode/tools/bin
curl -sL https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz -o /tmp/opencode/zig.tar.xz
echo 70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00 /tmp/opencode/zig.tar.xz | sha256sum -c -
tar -xJf /tmp/opencode/zig.tar.xz -C /tmp/opencode && mv /tmp/opencode/zig-x86_64-linux-0.16.0 /tmp/opencode/zig
printf '#!/bin/sh\nexec /tmp/opencode/zig/zig cc "$@"\n' > /tmp/opencode/tools/bin/cc
chmod +x /tmp/opencode/tools/bin/cc
```

On normal dev machines none of this is needed; plain `cargo build` works.
Do **not** commit a repo-level `.cargo/config.toml` forcing zig — it would
break everyone else.

## Commands

```sh
cargo fmt --check          # formatting gate
cargo test                 # unit tests (pure parsers only, no root needed)
cargo clippy --all-targets -- -D warnings   # must be warning-free
cargo build                # host build
cargo build --features gui # GUI build (needs Wayland/X11 dev libs on Linux)
cargo test --features gui  # GUI tests (same parsers, GUI feature checked)

# fully static musl binary in THIS sandbox (zig cc double-injects CRT and
# breaks _start, so link directly with rustup's bundled rust-lld):
SYSROOT=$(rustc --print sysroot)
RUSTFLAGS="-C linker-flavor=ld.lld -C linker=$SYSROOT/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld" \
    cargo build --release --target x86_64-unknown-linux-musl
```

All four gates must pass before finishing any task. Tests are deliberately
root-free; anything touching real syscalls (`drop_caches`, PSI trigger
registration, slabinfo reads) is exercised through parsers plus manual smoke
tests as root elsewhere.

## Conventions

- Errors: `anyhow` with `.context()` chains that name the exact file and the
  fix (usually "run as root"). User-facing hints beat bare io errors.
- All sizes are `u64` bytes internally; format at print time via
  `format_bytes`. Parse user input with `parse_size` (K/M/G/T suffixes).
- JSON output is hand-rolled `println!` + `json_escape`; keep keys stable and
  snake_case, every command emits exactly one line/document so pipelines can
  consume it.
- New parsers go in their own module with table-driven unit tests using
  captured real-world samples (see `parse_slabinfo`, `parse_psi`,
  `parse_meminfo` tests).
- Keep clap doc-comments terse: they become help text.
- No async, no threads beyond `std::thread::sleep` (GUI uses `std::thread::spawn` for leak scan and exec hooks only), deps stay minimal
  (anyhow, clap, clap_complete, libc; `eframe`/`egui_plot` behind `gui` feature).

## Privilege model

| Operation | Root needed? |
| --- | --- |
| status/psi/pss/grow/zram/oom/completions | no |
| clean (drop_caches), compact | yes |
| watch --psi (trigger registration) | yes |
| reclaim/limit (cgroup memory files) | yes + cgroup v2 delegation |
| slab (slabinfo read) | yes on most kernels |

## Release checklist

1. Bump version in `Cargo.toml`, update `CHANGELOG.md`.
2. Gates green (fmt/test/clippy), smoke test: `status`, `watch --once`,
   `completions bash`, `--json` variants parse with `python3 -m json.tool`.
   If `gui` changed, also `cargo check --features gui` and smoke `gui --help`.
3. Build static musl binaries x86_64 + aarch64 (CI does this on tags).
4. Update packaging versions if needed (`packaging/`).
