use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::HashMap;
use std::{fs, thread, time::Duration};

mod cgroup;
mod mem;
mod procs;
mod psi;
mod zram;

use mem::{format_bytes, parse_size, read_memory, Memory};

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
    /// Show physical and swap memory usage plus PSI pressure.
    Status,

    /// Show PSI memory pressure state.
    Psi {
        /// Read pressure from a cgroup directory instead of the whole system.
        #[arg(long, value_name = "PATH")]
        cgroup: Option<String>,
    },

    /// Flush clean page cache and reclaimable slab objects.
    Clean {
        /// Cache class to reclaim: page-cache, slab, or all.
        #[arg(long, value_enum, default_value = "all")]
        mode: CleanMode,

        /// Report the operation without changing system state.
        #[arg(long)]
        dry_run: bool,
    },

    /// Reclaim memory in a cgroup subtree instead of dropping caches globally.
    Reclaim {
        /// Target cgroup relative to the cgroup2 mount (default: current cgroup).
        #[arg(long, value_name = "PATH")]
        cgroup: Option<String>,

        /// Bytes to reclaim (suffixes K/M/G allowed); 0, max, or unset = best effort.
        #[arg(long, value_name = "BYTES")]
        bytes: Option<String>,

        /// Report the operation without changing system state.
        #[arg(long)]
        dry_run: bool,
    },

    /// Show or set cgroup v2 memory limits.
    Limit {
        #[command(subcommand)]
        action: LimitAction,
    },

    /// Clean automatically when used memory reaches threshold.
    Watch {
        /// Used-memory (or PSI avg10 with --psi) threshold, percent.
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

        /// Use PSI pressure events (some/full) instead of periodic polling.
        #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "full")]
        psi: Option<PsiMetric>,
    },

    /// Per-process memory attribution via smaps_rollup.
    Pss {
        /// PIDs to inspect (default: all processes, sorted by PSS).
        #[arg(value_name = "PID", num_args = 0..)]
        pids: Vec<i32>,

        /// Limit listing to the N largest processes.
        #[arg(long, default_value_t = 10)]
        top: usize,
    },

    /// Detect memory growth or leaks by sampling per-process PSS.
    Grow {
        /// Seconds between the two samples.
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
        interval: u64,

        /// Report only processes growing by at least this much (suffixes K/M/G allowed).
        #[arg(long, default_value = "32M", value_name = "BYTES")]
        min: String,
    },

    /// Swap and zram compression telemetry.
    Zram {
        /// Measure swap-in/out rates over this many seconds (0 = disabled).
        #[arg(long, default_value_t = 0, value_name = "SECS")]
        sample: u64,
    },
}

#[derive(Subcommand, Debug)]
enum LimitAction {
    /// Show memory limits of a cgroup (default: current cgroup).
    Show {
        /// Target cgroup relative to the cgroup2 mount.
        #[arg(value_name = "PATH")]
        path: Option<String>,
    },

    /// Set a memory limit: <path> <high|max> <bytes|max>.
    Set {
        /// Target cgroup relative to the cgroup2 mount.
        #[arg(value_name = "PATH")]
        path: String,

        /// Which limit to set.
        #[arg(value_enum)]
        kind: LimitKind,

        /// Size in bytes (suffixes K/M/G allowed) or max.
        #[arg(value_name = "BYTES")]
        value: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LimitKind {
    High,
    Max,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum PsiMetric {
    #[value(name = "some")]
    Some_,
    #[value(name = "full")]
    Full,
}

impl PsiMetric {
    fn name(self) -> &'static str {
        match self {
            Self::Some_ => "some",
            Self::Full => "full",
        }
    }
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
        CommandKind::Psi { cgroup } => print_psi(&cgroup, cli.json),
        CommandKind::Clean { mode, dry_run } => clean(mode, dry_run, cli.json),
        CommandKind::Reclaim {
            cgroup,
            bytes,
            dry_run,
        } => reclaim(cgroup.as_deref(), bytes.as_deref(), dry_run, cli.json),
        CommandKind::Limit { action } => limit(action, cli.json),
        CommandKind::Watch {
            threshold,
            interval,
            mode,
            once,
            dry_run,
            psi,
        } => watch(threshold, interval, mode, once, dry_run, psi, cli.json),
        CommandKind::Pss { pids, top } => pss(&pids, top, cli.json),
        CommandKind::Grow { interval, min } => grow(interval, &min, cli.json),
        CommandKind::Zram { sample } => zram(sample, cli.json),
    }
}

fn print_status(memory: Memory, json: bool) -> Result<()> {
    let pressure = psi::read_psi().ok();

    if json {
        let (some, full) = match pressure {
            Some(p) => (p.some, p.full),
            None => (psi::PsiCounter::default(), psi::PsiCounter::default()),
        };
        println!(
            "{{\"total_bytes\":{},\"used_bytes\":{},\"available_bytes\":{},\"cached_bytes\":{},\"buffers_bytes\":{},\"reclaimable_bytes\":{},\"used_percent\":{},\"swap_total_bytes\":{},\"swap_used_bytes\":{},\"psi_available\":{},\"psi_some_avg10_pct\":{:.2},\"psi_some_avg60_pct\":{:.2},\"psi_some_avg300_pct\":{:.2},\"psi_some_total_us\":{},\"psi_full_avg10_pct\":{:.2},\"psi_full_avg60_pct\":{:.2},\"psi_full_avg300_pct\":{:.2},\"psi_full_total_us\":{}}}",
            memory.total,
            memory.used(),
            memory.available,
            memory.cached,
            memory.buffers,
            memory.reclaimable,
            memory.used_percent(),
            memory.swap_total,
            memory.swap_used(),
            pressure.is_some(),
            some.avg10,
            some.avg60,
            some.avg300,
            some.total,
            full.avg10,
            full.avg60,
            full.avg300,
            full.total,
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
    match pressure {
        Some(p) => println!(
            "  pressure:   some {:.2}% / full {:.2}% (avg10)",
            p.some.avg10, p.full.avg10
        ),
        None => println!("  pressure:   unavailable"),
    }
    Ok(())
}

fn print_psi(cgroup_arg: &Option<String>, json: bool) -> Result<()> {
    let (scope, pressure) = match cgroup_arg {
        None => ("system".to_string(), psi::read_psi()?),
        Some(relative) => {
            let cg = cgroup::Cgroup::current()?;
            let dir = cg.resolve(relative);
            (dir.display().to_string(), psi::read_psi_cgroup(&dir)?)
        }
    };

    if json {
        println!(
            "{{\"scope\":\"{}\",\"some_avg10_pct\":{:.2},\"some_avg60_pct\":{:.2},\"some_avg300_pct\":{:.2},\"some_total_us\":{},\"full_avg10_pct\":{:.2},\"full_avg60_pct\":{:.2},\"full_avg300_pct\":{:.2},\"full_total_us\":{}}}",
            json_escape(&scope),
            pressure.some.avg10,
            pressure.some.avg60,
            pressure.some.avg300,
            pressure.some.total,
            pressure.full.avg10,
            pressure.full.avg60,
            pressure.full.avg300,
            pressure.full.total,
        );
    } else {
        println!("Pressure");
        println!("  scope:    {scope}");
        println!(
            "  some:     avg10 {:.2}%  avg60 {:.2}%  avg300 {:.2}%  total {:.1}s",
            pressure.some.avg10,
            pressure.some.avg60,
            pressure.some.avg300,
            pressure.some.total as f64 / 1e6
        );
        println!(
            "  full:     avg10 {:.2}%  avg60 {:.2}%  avg300 {:.2}%  total {:.1}s",
            pressure.full.avg10,
            pressure.full.avg60,
            pressure.full.avg300,
            pressure.full.total as f64 / 1e6
        );
    }
    Ok(())
}

fn clean(mode: CleanMode, dry_run: bool, json: bool) -> Result<()> {
    let before = read_memory()?;

    if dry_run {
        return print_clean(mode, before, before, true, json);
    }

    unsafe { libc::sync() };

    fs::write(mem::DROP_CACHES, mode.value().to_string()).with_context(|| {
        format!(
            "write {}; run as root, for example: sudo memreduct clean --mode {}",
            mem::DROP_CACHES,
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
    let reclaimed = before
        .reclaimable_caches()
        .saturating_sub(after.reclaimable_caches());
    if json {
        println!(
            "{{\"mode\":\"{}\",\"dry_run\":{},\"reclaimed_bytes\":{},\"used_before_bytes\":{},\"used_after_bytes\":{}}}",
            mode.name(),
            dry_run,
            reclaimed,
            before.used(),
            after.used()
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

fn reclaim(
    cgroup_arg: Option<&str>,
    bytes_text: Option<&str>,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let cg = cgroup::Cgroup::current()?;
    let dir = match cgroup_arg {
        None => cg.dir(),
        Some(relative) => cg.resolve(relative),
    };

    let reclaim_file = dir.join("memory.reclaim");
    if !reclaim_file.is_file() {
        bail!(
            "{} missing: memory controller not enabled for this cgroup (needs cgroup v2 delegation)",
            reclaim_file.display()
        );
    }

    let bytes = cgroup::parse_reclaim_bytes(bytes_text)?;
    let before = cgroup::field(&dir, "memory.current")?;

    if !dry_run {
        let content = match bytes {
            Some(bytes) => bytes.to_string(),
            None => "0".to_string(),
        };
        cgroup::write(&dir, "memory.reclaim", &content)?;
    }

    let after = cgroup::field(&dir, "memory.current")?;
    let reclaimed = before.saturating_sub(after);

    if json {
        println!(
            "{{\"cgroup\":\"{}\",\"dry_run\":{},\"before_bytes\":{},\"after_bytes\":{},\"reclaimed_bytes\":{}}}",
            json_escape(&dir.display().to_string()),
            dry_run,
            before,
            after,
            reclaimed
        );
    } else if dry_run {
        println!("would reclaim memory in {}", dir.display());
    } else {
        println!(
            "reclaimed {} in {} (current {} -> {})",
            format_bytes(reclaimed),
            dir.display(),
            format_bytes(before),
            format_bytes(after)
        );
    }
    Ok(())
}

fn limit(action: LimitAction, json: bool) -> Result<()> {
    let cg = cgroup::Cgroup::current()?;
    match action {
        LimitAction::Show { path } => limit_show(&cg, path.as_deref(), json),
        LimitAction::Set { path, kind, value } => limit_set(&cg, &path, kind, &value, json),
    }
}

fn limit_show(cg: &cgroup::Cgroup, path: Option<&str>, json: bool) -> Result<()> {
    let dir = match path {
        None => cg.dir(),
        Some(relative) => cg.resolve(relative),
    };

    let high = cgroup::field_opt(&dir, "memory.high")?;
    let max = cgroup::field_opt(&dir, "memory.max")?;
    let current = cgroup::field_opt(&dir, "memory.current")?
        .context(format!("missing memory.current in {}", dir.display()))?;

    let mut events = HashMap::new();
    if let Ok(text) = fs::read_to_string(dir.join("memory.events")) {
        for (key, value) in cgroup::parse_events(&text) {
            events.insert(key, value);
        }
    }
    let count = |key: &str| events.get(key).copied().unwrap_or(0);

    if json {
        let high_json = match high {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        };
        let max_json = match max {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        };
        println!(
            "{{\"cgroup\":\"{}\",\"memory_high\":{},\"memory_max\":{},\"memory_current\":{},\"events\":{{\"low\":{},\"high\":{},\"max\":{},\"oom\":{},\"oom_kill\":{},\"oom_group_kill\":{}}}}}",
            json_escape(&dir.display().to_string()),
            high_json,
            max_json,
            current,
            count("low"),
            count("high"),
            count("max"),
            count("oom"),
            count("oom_kill"),
            count("oom_group_kill")
        );
    } else {
        println!("cgroup: {}", dir.display());
        match high {
            Some(v) => println!("  high:    {}", format_bytes(v)),
            None => println!("  high:    max"),
        }
        match max {
            Some(v) => println!("  max:     {}", format_bytes(v)),
            None => println!("  max:     max"),
        }
        println!("  current: {}", format_bytes(current));
        println!(
            "  events:  low {} / high {} / max {} / oom {} / oom_kill {} / oom_group_kill {}",
            count("low"),
            count("high"),
            count("max"),
            count("oom"),
            count("oom_kill"),
            count("oom_group_kill")
        );
    }
    Ok(())
}

fn limit_set(
    cg: &cgroup::Cgroup,
    path: &str,
    kind: LimitKind,
    value: &str,
    json: bool,
) -> Result<()> {
    let dir = cg.resolve(path);
    let file = match kind {
        LimitKind::High => "memory.high",
        LimitKind::Max => "memory.max",
    };

    if !dir.join(file).is_file() {
        bail!(
            "{} missing: memory controller not enabled on {}",
            file,
            dir.display()
        );
    }

    let parsed = if value.trim() == "max" {
        None
    } else {
        Some(parse_size(value)?)
    };
    let content = match parsed {
        Some(bytes) => bytes.to_string(),
        None => "max".to_string(),
    };
    cgroup::write(&dir, file, &content)?;

    if json {
        let value_json = match parsed {
            Some(bytes) => bytes.to_string(),
            None => "null".to_string(),
        };
        println!(
            "{{\"cgroup\":\"{}\",\"limit\":\"{file}\",\"value_bytes\":{}}}",
            json_escape(&dir.display().to_string()),
            value_json
        );
    } else {
        let shown = match parsed {
            Some(bytes) => format_bytes(bytes),
            None => "unlimited".to_string(),
        };
        println!("set {file} of {} to {shown}", dir.display());
    }
    Ok(())
}

fn watch(
    threshold: u8,
    interval: u64,
    mode: CleanMode,
    once: bool,
    dry_run: bool,
    psi_metric: Option<PsiMetric>,
    json: bool,
) -> Result<()> {
    match psi_metric {
        Some(metric) => watch_psi(threshold, interval, mode, once, dry_run, metric, json),
        None => watch_poll(threshold, interval, mode, once, dry_run, json),
    }
}

fn watch_poll(
    threshold: u8,
    interval: u64,
    mode: CleanMode,
    once: bool,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    loop {
        let memory = match read_memory() {
            Ok(memory) => memory,
            Err(error) => {
                if once {
                    return Err(error);
                }
                eprintln!("error: {error:#}");
                thread::sleep(Duration::from_secs(interval));
                continue;
            }
        };
        if memory.used_percent() >= threshold {
            if let Err(error) = clean(mode, dry_run, json) {
                eprintln!("error: {error:#}");
            }
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

fn watch_psi(
    threshold: u8,
    interval: u64,
    mode: CleanMode,
    once: bool,
    dry_run: bool,
    metric: PsiMetric,
    json: bool,
) -> Result<()> {
    let window_us = interval.clamp(1, 10) * 1_000_000;
    let stall_us = window_us / 100 * u64::from(threshold);
    let file = psi::PsiFile::open(metric.name(), stall_us, window_us)?;
    let timeout = interval.saturating_mul(1000).min(i32::MAX as u64) as i32;

    loop {
        let event = file.wait_event(timeout)?;
        let pressure = file.read()?;
        let avg10 = match metric {
            PsiMetric::Some_ => pressure.some.avg10,
            PsiMetric::Full => pressure.full.avg10,
        };

        if event && avg10 >= threshold as f64 {
            if let Err(error) = clean(mode, dry_run, json) {
                eprintln!("error: {error:#}");
            }
        } else if !json {
            println!(
                "pressure {} {:.2}%; threshold {}%",
                metric.name(),
                avg10,
                threshold
            );
        }

        if once {
            return Ok(());
        }
    }
}

fn pss(pids: &[i32], top: usize, json: bool) -> Result<()> {
    let mut entries = Vec::new();
    if pids.is_empty() {
        for pid in procs::all_pids() {
            let Ok(comm) = procs::read_comm(pid) else {
                continue;
            };
            let Ok(mem) = procs::read_smaps_rollup(pid) else {
                continue;
            };
            entries.push(procs::ProcEntry { pid, comm, mem });
        }
    } else {
        for &pid in pids {
            let comm = procs::read_comm(pid)?;
            let mem = procs::read_smaps_rollup(pid)?;
            entries.push(procs::ProcEntry { pid, comm, mem });
        }
    }

    entries.sort_by_key(|entry| std::cmp::Reverse(entry.mem.pss));
    if pids.is_empty() {
        entries.truncate(top);
    }

    if json {
        print!("[");
        for (index, entry) in entries.iter().enumerate() {
            if index > 0 {
                print!(",");
            }
            print!(
                "{{\"pid\":{},\"comm\":\"{}\",\"rss_bytes\":{},\"pss_bytes\":{},\"pss_anon_bytes\":{},\"pss_file_bytes\":{},\"swap_bytes\":{}}}",
                entry.pid,
                json_escape(&entry.comm),
                entry.mem.rss,
                entry.mem.pss,
                entry.mem.pss_anon,
                entry.mem.pss_file,
                entry.mem.swap
            );
        }
        println!("]");
    } else {
        if entries.is_empty() {
            println!("no accessible processes (run as root for full visibility)");
            return Ok(());
        }
        println!(
            "  {:>7}  {:<24} {:>10} {:>10} {:>10}",
            "PID", "COMM", "RSS", "PSS", "SWAP"
        );
        for entry in &entries {
            let comm: String = entry.comm.chars().take(24).collect();
            println!(
                "  {:>7}  {:<24} {:>10} {:>10} {:>10}",
                entry.pid,
                comm,
                format_bytes(entry.mem.rss),
                format_bytes(entry.mem.pss),
                format_bytes(entry.mem.swap)
            );
        }
    }
    Ok(())
}

fn grow(interval: u64, min_text: &str, json: bool) -> Result<()> {
    let min = parse_size(min_text)?;
    let before = snapshot_pss();
    thread::sleep(Duration::from_secs(interval));
    let after = snapshot_pss();

    let mut gains: Vec<(i32, u64, u64, u64, String)> = Vec::new();
    for (pid, starttime, comm, pss) in &after {
        let delta = match before
            .iter()
            .find(|(p, s, _, _)| *p == *pid && *s == *starttime)
        {
            Some((_, _, _, before_pss)) => pss.saturating_sub(*before_pss),
            None => *pss,
        };
        if delta >= min {
            gains.push((*pid, *starttime, *pss, delta, comm.clone()));
        }
    }
    gains.sort_by_key(|gain| std::cmp::Reverse(gain.3));

    if json {
        print!(
            "{{\"interval_seconds\":{},\"min_bytes\":{},\"growers\":[",
            interval, min
        );
        for (index, (pid, _, pss, delta, comm)) in gains.iter().enumerate() {
            if index > 0 {
                print!(",");
            }
            print!(
                "{{\"pid\":{},\"comm\":\"{}\",\"pss_bytes\":{},\"delta_bytes\":{},\"rate_bytes_per_sec\":{:.1}}}",
                pid,
                json_escape(comm),
                pss,
                delta,
                *delta as f64 / interval as f64
            );
        }
        println!("]}}");
    } else {
        if gains.is_empty() {
            println!(
                "no significant growth over {interval}s (min {})",
                format_bytes(min)
            );
            return Ok(());
        }
        println!("growth over {interval}s (min {}):", format_bytes(min));
        for (pid, _, _, delta, comm) in &gains {
            let comm: String = comm.chars().take(24).collect();
            println!(
                "  {:>7}  {:<24}  +{}  ({} /s)",
                pid,
                comm,
                format_bytes(*delta),
                format_bytes((*delta as f64 / interval as f64) as u64)
            );
        }
    }
    Ok(())
}

fn snapshot_pss() -> Vec<(i32, u64, String, u64)> {
    let mut snapshot = Vec::new();
    for pid in procs::all_pids() {
        let Ok(starttime) = procs::read_starttime(pid) else {
            continue;
        };
        let Ok(comm) = procs::read_comm(pid) else {
            continue;
        };
        let Ok(mem) = procs::read_smaps_rollup(pid) else {
            continue;
        };
        snapshot.push((pid, starttime, comm, mem.pss));
    }
    snapshot
}

fn zram(sample: u64, json: bool) -> Result<()> {
    let memory = read_memory()?;
    let devices = zram::read_zram_devices();

    let (pages_in, pages_out, sampled) = if sample > 0 {
        let (before_in, before_out) = zram::read_vmstat()?;
        thread::sleep(Duration::from_secs(sample));
        let (after_in, after_out) = zram::read_vmstat()?;
        (
            after_in.saturating_sub(before_in),
            after_out.saturating_sub(before_out),
            true,
        )
    } else {
        (0, 0, false)
    };

    if json {
        print!(
            "{{\"swap_total_bytes\":{},\"swap_free_bytes\":{},\"swap_used_bytes\":{},\"sample_seconds\":{},\"pswpin_pages_per_sec\":{},\"pswpout_pages_per_sec\":{},\"zram\":[",
            memory.swap_total,
            memory.swap_free,
            memory.swap_used(),
            if sampled { sample } else { 0 },
            if sampled {
                (pages_in as f64 / sample as f64).round() as u64
            } else {
                0
            },
            if sampled {
                (pages_out as f64 / sample as f64).round() as u64
            } else {
                0
            }
        );
        for (index, device) in devices.iter().enumerate() {
            if index > 0 {
                print!(",");
            }
            print!(
                "{{\"name\":\"{}\",\"disksize_bytes\":{},\"orig_bytes\":{},\"compressed_bytes\":{},\"ratio\":{:.2},\"mem_used_bytes\":{},\"mem_limit_bytes\":{},\"mem_used_max_bytes\":{},\"same_pages\":{},\"huge_pages\":{}}}",
                device.name,
                device.disksize,
                device.orig,
                device.compressed,
                device.ratio(),
                device.mem_used,
                device.mem_limit,
                device.mem_used_max,
                device.same_pages,
                device.huge_pages
            );
        }
        println!("]}}");
    } else {
        println!("Swap");
        println!(
            "  total:    {} / used {}",
            format_bytes(memory.swap_total),
            format_bytes(memory.swap_used())
        );
        if sampled {
            let page = zram::page_size();
            println!(
                "  thrash:   {} KiB/s in, {} KiB/s out ({}s window)",
                pages_in.saturating_mul(page) / 1024 / sample,
                pages_out.saturating_mul(page) / 1024 / sample,
                sample
            );
        }
        if devices.is_empty() {
            println!("Zram");
            println!("  no zram devices");
        } else {
            println!("Zram");
            for device in &devices {
                println!(
                    "  {}:   {} disk, orig {} -> comp {} (ratio {:.1}x), mem {}, same_pages {}, huge_pages {}",
                    device.name,
                    format_bytes(device.disksize),
                    format_bytes(device.orig),
                    format_bytes(device.compressed),
                    device.ratio(),
                    format_bytes(device.mem_used),
                    device.same_pages,
                    device.huge_pages
                );
            }
        }
    }
    Ok(())
}

fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_json_strings() {
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(json_escape("line\nbreak"), "line\\nbreak");
        assert_eq!(json_escape("plain"), "plain");
    }

    #[test]
    fn cleans_mode_values() {
        assert_eq!(CleanMode::PageCache.value(), 1);
        assert_eq!(CleanMode::Slab.value(), 2);
        assert_eq!(CleanMode::All.value(), 3);
    }
}
