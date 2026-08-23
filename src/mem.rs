use anyhow::{bail, Context, Result};
use std::fs;

pub const MEMINFO: &str = "/proc/meminfo";
pub const DROP_CACHES: &str = "/proc/sys/vm/drop_caches";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub cached: u64,
    pub buffers: u64,
    pub reclaimable: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl Memory {
    pub fn used(self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn used_percent(self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        ((self.used() * 100) / self.total).min(100) as u8
    }

    pub fn swap_used(self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }

    pub fn reclaimable_caches(self) -> u64 {
        self.cached + self.buffers + self.reclaimable
    }
}

pub fn read_memory() -> Result<Memory> {
    let text = fs::read_to_string(MEMINFO).context("read /proc/meminfo")?;
    parse_meminfo(&text)
}

pub fn parse_meminfo(text: &str) -> Result<Memory> {
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

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
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

pub fn parse_size(text: &str) -> Result<u64> {
    let mut t = text.trim();
    for _ in 0..2 {
        if let Some(rest) = t.strip_suffix(['b', 'B', 'i', 'I']) {
            t = rest;
        }
    }
    let mut mult = 1u64;
    if let Some(last) = t.chars().last() {
        if matches!(last, 'k' | 'K' | 'm' | 'M' | 'g' | 'G') {
            mult = match last {
                'k' | 'K' => 1024,
                'm' | 'M' => 1024 * 1024,
                _ => 1024 * 1024 * 1024,
            };
            t = &t[..t.len() - 1];
        }
    }
    let t = t.trim();
    if t.is_empty() {
        bail!("missing number in size {text:?}");
    }
    let number: u64 = t
        .parse()
        .with_context(|| format!("invalid size {text:?}"))?;
    Ok(number.saturating_mul(mult))
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

    #[test]
    fn parses_sizes_with_suffixes() {
        assert_eq!(parse_size("512").unwrap(), 512);
        assert_eq!(parse_size("1K").unwrap(), 1024);
        assert_eq!(parse_size("2m").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_size("3G").unwrap(), 3 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("4KiB").unwrap(), 4 * 1024);
        assert_eq!(parse_size("5MB").unwrap(), 5 * 1024 * 1024);
        assert_eq!(parse_size("6MiB").unwrap(), 6 * 1024 * 1024);
        assert_eq!(parse_size("0").unwrap(), 0);
        assert!(parse_size("abc").is_err());
        assert!(parse_size("").is_err());
    }
}
