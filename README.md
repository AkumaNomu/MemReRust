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
memreduct --json status
```

`clean` runs `sync`, then writes Linux-native `drop_caches`. Root required.
Linux cannot safely expose Windows working-set, standby-list, or registry-cache operations through this interface.

Cache dropping is mainly for testing/debugging. Linux reclaims caches automatically; forced cleanup can hurt I/O performance.

GPL-3.0-only. Original project: <https://github.com/henrypp/memreduct>.
