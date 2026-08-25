# memreduct — cheat sheet

Paste these. Replace `my.slice` / threshold numbers for your box.

```sh
memreduct status
memreduct --json status | jq .

sudo memreduct clean --dry-run
sudo memreduct clean                  # all
sudo memreduct compact --dry-run

memreduct pss --top 10
sudo memreduct pss --top 20
memreduct oom --top 10
sudo memreduct slab --top 10
memreduct zram --sample 10

sudo memreduct grow --interval 60 --min 32M

# watch
memreduct watch --once --threshold 90 --dry-run
sudo memreduct watch --threshold 90 --interval 30 --cooldown 300
sudo memreduct watch --psi some --threshold 8 --mode page-cache
sudo memreduct watch --threshold 90 --swap-threshold 70 --exec 'notify-send cleaned'

# cgroup
memreduct limit show
sudo memreduct limit set my.slice high 2G
sudo memreduct limit set my.slice max 4G
sudo memreduct reclaim --cgroup my.slice --dry-run
sudo memreduct reclaim --cgroup my.slice --bytes 512M

# psi
memreduct psi
sudo memreduct psi --cgroup my.slice

# json + jq
memreduct --json pss --top 5 | jq -r '.[] | "\(.pid) \(.comm) \(.pss_bytes)"'
memreduct --json status | jq .used_percent

# gui (needs --features gui build)
memreduct gui
memreduct --gui
memreduct              # no args → gui if built with gui
cargo build --features gui --release && ./target/release/memreduct gui
```

Every command supports `--json`. Needs root: `clean`, `compact`, `watch --psi`, `reclaim`, `limit set`, `slab`.
