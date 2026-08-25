use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Cgroup {
    mount: PathBuf,
    path: PathBuf,
}

impl Cgroup {
    pub fn current() -> Result<Self> {
        let info =
            fs::read_to_string("/proc/self/mountinfo").context("read /proc/self/mountinfo")?;
        let mut mount = None;
        for line in info.lines() {
            let Some((head, tail)) = line.split_once(" - ") else {
                continue;
            };
            let tail: Vec<&str> = tail.split_whitespace().collect();
            if tail.first() == Some(&"cgroup2") {
                let head: Vec<&str> = head.split_whitespace().collect();
                mount = head.get(4).map(PathBuf::from);
                break;
            }
        }
        let mount = mount.context(
            "cgroup v2 unified hierarchy not found (is the system running legacy cgroup v1?)",
        )?;

        let own = fs::read_to_string("/proc/self/cgroup").context("read /proc/self/cgroup")?;
        let path = own
            .lines()
            .filter_map(|line| {
                line.split_once("::")
                    .map(|(_, path)| PathBuf::from(path.trim()))
            })
            .next()
            .context("no cgroup v2 entry in /proc/self/cgroup (legacy cgroup v1 systems are not supported)")?;

        if !mount.join(&path).is_dir() {
            bail!("cgroup directory {} missing", mount.join(&path).display());
        }
        Ok(Cgroup { mount, path })
    }

    pub fn dir(&self) -> PathBuf {
        self.mount.join(&self.path)
    }

    pub fn resolve(&self, relative: &str) -> PathBuf {
        self.mount.join(relative)
    }
}

pub fn field_opt(dir: &Path, file: &str) -> Result<Option<u64>> {
    match fs::read_to_string(dir.join(file)) {
        Ok(text) => {
            let text = text.trim();
            if text == "max" {
                Ok(None)
            } else {
                text.parse()
                    .map(Some)
                    .with_context(|| format!("invalid {file} in {}", dir.display()))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {file} in {}", dir.display())),
    }
}

pub fn field(dir: &Path, file: &str) -> Result<u64> {
    field_opt(dir, file)?.context(format!("missing {file} in {}", dir.display()))
}

pub fn write(dir: &Path, file: &str, content: &str) -> Result<()> {
    fs::write(dir.join(file), content)
        .with_context(|| format!("write {file} = {content} in {}", dir.display()))
}

pub fn parse_events(text: &str) -> Vec<(String, u64)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?.to_string();
            let count: u64 = fields.next()?.parse().ok()?;
            Some((name, count))
        })
        .collect()
}

pub fn parse_reclaim_bytes(text: Option<&str>) -> Result<Option<u64>> {
    match text {
        None => Ok(None),
        Some(text) => {
            let text = text.trim();
            if text.is_empty() || text == "0" || text == "max" {
                Ok(None)
            } else {
                crate::mem::parse_size(text).map(Some)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_memory_events() {
        let events = parse_events("low 0\nhigh 12\nmax 0\noom 0\noom_kill 0\n");
        assert!(events
            .iter()
            .any(|(key, value)| key == "high" && *value == 12));
        assert!(events.iter().any(|(key, _)| key == "oom_kill"));
    }

    #[test]
    fn reclaim_bytes_specials() {
        assert!(parse_reclaim_bytes(None).unwrap().is_none());
        assert!(parse_reclaim_bytes(Some("0")).unwrap().is_none());
        assert!(parse_reclaim_bytes(Some("max")).unwrap().is_none());
        assert!(parse_reclaim_bytes(Some("  ")).unwrap().is_none());
        assert_eq!(
            parse_reclaim_bytes(Some("64M")).unwrap(),
            Some(64 * 1024 * 1024)
        );
    }
}
