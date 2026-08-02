use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::{fs, process::Command, thread, time::Duration};

const MEMINFO: &str = "/proc/meminfo";
const DROP_CACHES: &str = "/proc/sys/vm/drop_caches";

#[derive(Parser, Debug)]
#[command(
    name = "memreduct",
    version,
    about = "Monitor and reclaim Linux memory caches"
)]
struct Cli {
    /// Emit JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand, Debug)]
enum CommandKind {
    /// Show physical and swap memory usage.
    Status,

    /// Flush clean page cache and reclaimable slab objects.
    Clean {
        /// Cache class to reclaim: page-cache, slab, or all.
        #[arg(long, value_enum, default_value = "all")]
        mode: CleanMode,

        /// Report the operation without changing system state.
        #[arg(long)]
        dry_run: bool,
    },

    /// Clean automatically when used memory reaches threshold.
    Watch {
        /// Used-memory threshold, percent.
        #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u8).range(1..=100))]
        threshold: u8,

        /// Seconds between checks.
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
        interval: u64,

        /// Cache class to reclaim: page-cache, slab, or all.
        #[arg(long, value_enum, default_value = "all")]
        mode: CleanMode,

        /// Check once, then exit.
        #[arg(long)]
        once: bool,

        /// Report planned cleanups without changing system state.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CleanMode {
    PageCache,
    Slab,
    All,
}

impl CleanMode {
    fn value(self) -> u8 {
        match self {
            Self::PageCache => 1,
            Self::Slab => 2,
            Self::All => 3,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::PageCache => "page-cache",
            Self::Slab => "slab",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Memory {
    total: u64,
    available: u64,
    cached: u64,
    buffers: u64,
    reclaimable: u64,
    swap_total: u64,
    swap_free: u64,
}

impl Memory {
    fn used(self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    fn used_percent(self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        ((self.used() * 100) / self.total).min(100) as u8
    }

    fn swap_used(self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        CommandKind::Status => print_status(read_memory()?, cli.json),
        CommandKind::Clean { mode, dry_run } => clean(mode, dry_run, cli.json),
        CommandKind::Watch {
            threshold,
            interval,
            mode,
            once,
            dry_run,
        } => watch(threshold, interval, mode, once, dry_run, cli.json),
    }
}

fn read_memory() -> Result<Memory> {
    let text = fs::read_to_string(MEMINFO).context("read /proc/meminfo")?;
    parse_meminfo(&text)
}

fn parse_meminfo(text: &str) -> Result<Memory> {
    let mut memory = Memory::default();

    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let mut fields = value.split_whitespace();
        let number: u64 = fields
            .next()
            .with_context(|| format!("missing value for {key}"))?
            .parse()
            .with_context(|| format!("invalid value for {key}"))?;
        let bytes = match fields.next() {
            Some("kB") => number.saturating_mul(1024),
            Some("MB") => number.saturating_mul(1024 * 1024),
            _ => number,
        };

        match key {
            "MemTotal" => memory.total = bytes,
            "MemAvailable" => memory.available = bytes,
            "Cached" => memory.cached = bytes,
            "Buffers" => memory.buffers = bytes,
            "SReclaimable" => memory.reclaimable = bytes,
            "SwapTotal" => memory.swap_total = bytes,
            "SwapFree" => memory.swap_free = bytes,
            _ => {}
        }
    }

    if memory.total == 0 {
        bail!("/proc/meminfo has no MemTotal");
    }
    Ok(memory)
}

fn print_status(memory: Memory, json: bool) -> Result<()> {
    if json {
        println!(
            "{{\"total_bytes\":{},\"used_bytes\":{},\"available_bytes\":{},\"cached_bytes\":{},\"buffers_bytes\":{},\"reclaimable_bytes\":{},\"used_percent\":{},\"swap_total_bytes\":{},\"swap_used_bytes\":{}}}",
            memory.total,
            memory.used(),
            memory.available,
            memory.cached,
            memory.buffers,
            memory.reclaimable,
            memory.used_percent(),
            memory.swap_total,
            memory.swap_used(),
        );
        return Ok(());
    }

    println!("Memory");
    println!("  total:      {}", format_bytes(memory.total));
    println!(
        "  used:       {} ({}%)",
        format_bytes(memory.used()),
        memory.used_percent()
    );
    println!("  available:  {}", format_bytes(memory.available));
    println!("  cached:     {}", format_bytes(memory.cached));
    println!("  buffers:    {}", format_bytes(memory.buffers));
    println!("  reclaimable: {}", format_bytes(memory.reclaimable));
    println!(
        "  swap:       {} / {}",
        format_bytes(memory.swap_used()),
        format_bytes(memory.swap_total)
    );
    Ok(())
}

fn clean(mode: CleanMode, dry_run: bool, json: bool) -> Result<()> {
    let before = read_memory()?;

    if dry_run {
        return print_clean(mode, before, before, true, json);
    }

    let sync = Command::new("sync")
        .status()
        .context("run sync before cache cleanup")?;
    if !sync.success() {
        bail!("sync failed with status {sync}");
    }

    fs::write(DROP_CACHES, mode.value().to_string()).with_context(|| {
        format!(
            "write {DROP_CACHES}; run as root, for example: sudo memreduct clean --mode {}",
            mode.name()
        )
    })?;

    let after = read_memory()?;
    print_clean(mode, before, after, false, json)
}

fn print_clean(
    mode: CleanMode,
    before: Memory,
    after: Memory,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let reclaimed = before.available.saturating_sub(after.available);
    if json {
        println!(
            "{{\"mode\":\"{}\",\"dry_run\":{},\"reclaimed_bytes\":{},\"used_before_bytes\":{},\"used_after_bytes\":{}}}",
            mode.name(), dry_run, reclaimed, before.used(), after.used()
        );
    } else if dry_run {
        println!("would clean {}", mode.name());
    } else {
        println!(
            "cleaned {}; reclaimed {}",
            mode.name(),
            format_bytes(reclaimed)
        );
    }
    Ok(())
}

fn watch(
    threshold: u8,
    interval: u64,
    mode: CleanMode,
    once: bool,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    loop {
        let memory = read_memory()?;
        if memory.used_percent() >= threshold {
            clean(mode, dry_run, json)?;
        } else if !json {
            println!(
                "memory {}%; threshold {}%",
                memory.used_percent(),
                threshold
            );
        }

        if once {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(interval));
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_meminfo() {
        let memory = parse_meminfo(
            "MemTotal:       1024 kB\nMemAvailable:    256 kB\nCached:          128 kB\nBuffers:          64 kB\nSReclaimable:     32 kB\nSwapTotal:       512 kB\nSwapFree:        256 kB\n",
        )
        .unwrap();

        assert_eq!(memory.total, 1024 * 1024);
        assert_eq!(memory.available, 256 * 1024);
        assert_eq!(memory.used_percent(), 75);
        assert_eq!(memory.swap_used(), 256 * 1024);
    }

    #[test]
    fn rejects_missing_total() {
        assert!(parse_meminfo("MemAvailable: 1 kB\n").is_err());
    }
}
