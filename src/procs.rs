use anyhow::{bail, Context, Result};
use std::fs;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcMem {
    pub rss: u64,
    pub pss: u64,
    pub pss_anon: u64,
    pub pss_file: u64,
    pub swap: u64,
}

#[derive(Clone, Debug)]
pub struct ProcEntry {
    pub pid: i32,
    pub comm: String,
    pub mem: ProcMem,
}

pub fn all_pids() -> Vec<i32> {
    let mut pids = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(pid) = name.parse() {
                    pids.push(pid);
                }
            }
        }
    }
    pids.sort_unstable();
    pids
}

pub fn read_comm(pid: i32) -> Result<String> {
    let text = fs::read_to_string(format!("/proc/{pid}/comm"))
        .with_context(|| format!("read comm of pid {pid}"))?;
    Ok(text.trim().to_string())
}

pub fn read_starttime(pid: i32) -> Result<u64> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("read stat of pid {pid}"))?;
    parse_starttime(&text)
}

pub fn parse_starttime(text: &str) -> Result<u64> {
    let Some(close) = text.rfind(')') else {
        bail!("malformed /proc stat");
    };
    let fields: Vec<&str> = text[close + 1..].split_whitespace().collect();
    fields
        .get(19)
        .context("stat lacks field 22")?
        .parse()
        .context("invalid starttime")
}

pub fn read_smaps_rollup(pid: i32) -> Result<ProcMem> {
    let text = fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))
        .with_context(|| format!("read smaps_rollup of pid {pid}"))?;
    parse_smaps_rollup(&text)
}

pub fn parse_smaps_rollup(text: &str) -> Result<ProcMem> {
    let mut mem = ProcMem::default();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(value) = rest.split_whitespace().next() else {
            continue;
        };
        let Ok(kbytes) = value.parse::<u64>() else {
            continue;
        };
        let bytes = kbytes * 1024;
        match name {
            "Rss" => mem.rss = bytes,
            "Pss" => mem.pss = bytes,
            "Pss_Anon" => mem.pss_anon = bytes,
            "Pss_File" => mem.pss_file = bytes,
            "Swap" => mem.swap = bytes,
            _ => {}
        }
    }
    Ok(mem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_smaps_rollup() {
        let mem = parse_smaps_rollup(
            "Rss: 5808 kB\nPss: 4100 kB\nPss_Anon: 3600 kB\nPss_File: 500 kB\nSwap: 64 kB\n",
        )
        .unwrap();
        assert_eq!(mem.rss, 5808 * 1024);
        assert_eq!(mem.pss, 4100 * 1024);
        assert_eq!(mem.pss_anon, 3600 * 1024);
        assert_eq!(mem.pss_file, 500 * 1024);
        assert_eq!(mem.swap, 64 * 1024);
    }

    #[test]
    fn parses_starttime_after_paren_comm() {
        let stat = "123 (my proc (weird)) S 1 123 123 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 1743 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert_eq!(parse_starttime(stat).unwrap(), 1743);
    }
}
