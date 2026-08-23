use anyhow::{bail, Context, Result};
use std::ffi::CString;
use std::fs;
use std::os::fd::RawFd;
use std::path::Path;

pub const PRESSURE_SYSTEM: &str = "/proc/pressure/memory";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PsiCounter {
    pub avg10: f64,
    pub avg60: f64,
    pub avg300: f64,
    pub total: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Psi {
    pub some: PsiCounter,
    pub full: PsiCounter,
}

pub fn parse_psi(text: &str) -> Result<Psi> {
    let mut psi = Psi::default();
    let mut have_some = false;
    let mut have_full = false;

    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let Some(kind) = fields.next() else {
            continue;
        };
        let mut counter = PsiCounter::default();
        for field in fields {
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            match key {
                "avg10" => {
                    counter.avg10 = value
                        .parse()
                        .with_context(|| format!("invalid avg10 {value}"))?
                }
                "avg60" => {
                    counter.avg60 = value
                        .parse()
                        .with_context(|| format!("invalid avg60 {value}"))?
                }
                "avg300" => {
                    counter.avg300 = value
                        .parse()
                        .with_context(|| format!("invalid avg300 {value}"))?
                }
                "total" => {
                    counter.total = value
                        .parse()
                        .with_context(|| format!("invalid total {value}"))?
                }
                _ => {}
            }
        }
        match kind {
            "some" => {
                psi.some = counter;
                have_some = true;
            }
            "full" => {
                psi.full = counter;
                have_full = true;
            }
            _ => {}
        }
    }

    if !have_some {
        bail!("pressure file has no 'some' line");
    }
    if !have_full {
        bail!("pressure file has no 'full' line");
    }
    Ok(psi)
}

pub fn read_psi() -> Result<Psi> {
    let text = fs::read_to_string(PRESSURE_SYSTEM).context("read /proc/pressure/memory")?;
    parse_psi(&text)
}

pub fn read_psi_cgroup(dir: &Path) -> Result<Psi> {
    let path = dir.join("memory.pressure");
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    parse_psi(&text)
}

pub struct PsiFile {
    fd: RawFd,
}

impl PsiFile {
    pub fn open(metric: &str, stall_us: u64, window_us: u64) -> Result<Self> {
        let path = CString::new(PRESSURE_SYSTEM).expect("no NUL in psi path");
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            bail!(
                "open {}: {} (PSI triggers need write access; run as root)",
                PRESSURE_SYSTEM,
                error
            );
        }

        let file = PsiFile { fd };
        if let Err(error) = file.register_trigger(metric, stall_us, window_us) {
            unsafe { libc::close(fd) };
            return Err(error);
        }
        Ok(file)
    }

    fn register_trigger(&self, metric: &str, stall_us: u64, window_us: u64) -> Result<()> {
        let trigger = format!("{metric} {stall_us} {window_us}\0");
        let ret = unsafe { libc::write(self.fd, trigger.as_ptr().cast(), trigger.len()) };
        if ret < 0 {
            let error = std::io::Error::last_os_error();
            let hint = match error.raw_os_error() {
                Some(libc::EBUSY) => "; another trigger owns this pressure file",
                Some(libc::EINVAL) => "; window must be between 500ms and 10s",
                _ => "",
            };
            return Err(error).with_context(|| {
                format!("write PSI trigger '{metric} {stall_us} {window_us}' to {PRESSURE_SYSTEM}{hint}")
            });
        }
        if ret as usize != trigger.len() {
            bail!("short write registering PSI trigger on {PRESSURE_SYSTEM}");
        }
        Ok(())
    }

    pub fn wait_event(&self, timeout_ms: i32) -> Result<bool> {
        let mut fds = [libc::pollfd {
            fd: self.fd,
            events: libc::POLLPRI | libc::POLLERR,
            revents: 0,
        }];
        let ret = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
        if ret < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                return Ok(false);
            }
            return Err(error).context("poll /proc/pressure/memory");
        }
        Ok(ret > 0 && fds[0].revents != 0)
    }

    pub fn read(&self) -> Result<Psi> {
        let mut buf = [0u8; 4096];
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            return Err(std::io::Error::last_os_error()).context("read /proc/pressure/memory");
        }
        let n = n as usize;
        if n == 0 {
            return Ok(Psi::default());
        }
        let text = std::str::from_utf8(&buf[..n]).context("pressure file not utf-8")?;
        parse_psi(text)
    }
}

impl Drop for PsiFile {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pressure_file() {
        let psi = parse_psi(
            "some avg10=0.12 avg60=0.08 avg300=0.04 total=1234567\nfull avg10=0.01 avg60=0.00 avg300=0.00 total=42\n",
        )
        .unwrap();
        assert!((psi.some.avg10 - 0.12).abs() < 1e-9);
        assert_eq!(psi.some.total, 1234567);
        assert!((psi.full.avg10 - 0.01).abs() < 1e-9);
        assert_eq!(psi.full.total, 42);
    }

    #[test]
    fn rejects_truncated_pressure_file() {
        assert!(parse_psi("some avg10=0.00\n").is_err());
    }
}
