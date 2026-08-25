use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZramDevice {
    pub name: String,
    pub disksize: u64,
    pub orig: u64,
    pub compressed: u64,
    pub mem_used: u64,
    pub mem_limit: u64,
    pub mem_used_max: u64,
    pub same_pages: u64,
    pub huge_pages: u64,
}

impl ZramDevice {
    pub fn ratio(&self) -> f64 {
        if self.compressed == 0 {
            0.0
        } else {
            self.orig as f64 / self.compressed as f64
        }
    }

    fn apply_mm_stat(&mut self, text: &str) {
        let values: Vec<u64> = text
            .split_whitespace()
            .filter_map(|token| token.parse().ok())
            .collect();
        self.orig = values.first().copied().unwrap_or(0);
        self.compressed = values.get(1).copied().unwrap_or(0);
        self.mem_used = values.get(2).copied().unwrap_or(0);
        self.mem_limit = values.get(3).copied().unwrap_or(0);
        self.mem_used_max = values.get(4).copied().unwrap_or(0);
        self.same_pages = values.get(5).copied().unwrap_or(0);
        self.huge_pages = values.get(7).copied().unwrap_or(0);
    }
}

pub fn page_size() -> u64 {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 {
        size as u64
    } else {
        4096
    }
}

pub fn read_zram_devices() -> Vec<ZramDevice> {
    let mut devices = Vec::new();
    let Ok(blocks) = fs::read_dir("/sys/block") else {
        return devices;
    };
    let mut paths: Vec<PathBuf> = blocks.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let Some(name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
        else {
            continue;
        };
        let Some(index) = name.strip_prefix("zram") else {
            continue;
        };
        if index.is_empty() || index.bytes().any(|b| !b.is_ascii_digit()) {
            continue;
        }
        devices.push(read_device(&name, &path));
    }
    devices
}

fn read_device(name: &str, path: &Path) -> ZramDevice {
    let mut device = ZramDevice {
        name: name.to_string(),
        ..Default::default()
    };
    device.disksize = read_u64(&path.join("disksize")).unwrap_or(0);
    if let Ok(text) = fs::read_to_string(path.join("mm_stat")) {
        device.apply_mm_stat(&text);
    }
    device
}

fn read_u64(path: &Path) -> anyhow::Result<u64> {
    fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?
        .trim()
        .parse()
        .with_context(|| format!("invalid value in {}", path.display()))
}

pub fn read_vmstat() -> anyhow::Result<(u64, u64)> {
    let text = fs::read_to_string("/proc/vmstat").context("read /proc/vmstat")?;
    let mut pages_in = 0;
    let mut pages_out = 0;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("pswpin ") {
            pages_in = rest.parse().context("invalid pswpin")?;
        } else if let Some(rest) = line.strip_prefix("pswpout ") {
            pages_out = rest.parse().context("invalid pswpout")?;
        }
    }
    Ok((pages_in, pages_out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mm_stat_bytes() {
        let mut device = ZramDevice::default();
        device.apply_mm_stat("3221225472 1073741824 1073741824 0 1610612736 512 12 3");
        assert_eq!(device.orig, 3221225472);
        assert_eq!(device.compressed, 1073741824);
        assert_eq!(device.mem_used, 1073741824);
        assert_eq!(device.mem_used_max, 1610612736);
        assert_eq!(device.same_pages, 512);
        assert_eq!(device.huge_pages, 3);
        assert!((device.ratio() - 3.0).abs() < 1e-9);
    }
}
