use std::os::unix::process::CommandExt;
use std::path::Path;

/// Process control abstraction so management logic can be tested without
/// spawning real processes (real impl = std::process).
pub trait Runner: Send + Sync {
    fn spawn(&self, cmd: &[String]) -> Result<u32, std::io::Error>;
    fn is_running(&self, pid: u32) -> bool;
    fn kill(&self, pid: u32) -> std::result::Result<(), std::io::Error>;
}

#[derive(Debug, Clone, Default)]
pub struct RealRunner;

impl Runner for RealRunner {
    fn spawn(&self, cmd: &[String]) -> std::result::Result<u32, std::io::Error> {
        if cmd.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "empty command",
            ));
        }
        let mut cmd_c = std::process::Command::new(&cmd[0]);
        cmd_c
            .args(&cmd[1..])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null());
        unsafe {
            cmd_c.pre_exec(|| {
                // detach from the caller's process group so services
                // survive the parent (terminal/daemon) exiting
                libc::setsid();
                Ok(())
            });
        }
        cmd_c.spawn().map(|c| c.id())
    }
    fn is_running(&self, pid: u32) -> bool {
        std::path::Path::new(format!("/proc/{pid}").as_str()).exists()
    }
    fn kill(&self, pid: u32) -> std::result::Result<(), std::io::Error> {
        std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .output()?;
        Ok(())
    }
}

pub fn write_pid_file(dir: &Path, name: &str, pid: u32) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("{name}.pid")), pid.to_string())
}

pub fn read_pid_file(dir: &Path, name: &str) -> Result<Option<u32>, std::io::Error> {
    let p = dir.join(format!("{name}.pid"));
    match std::fs::read_to_string(&p) {
        Ok(s) => Ok(s.trim().parse::<u32>().ok()),
        Err(_) => Ok(None),
    }
}

pub fn remove_pid_file(dir: &Path, name: &str) {
    let _ = std::fs::remove_file(dir.join(format!("{name}.pid")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_file_round_trip() {
        let t = tempfile::tempdir().unwrap();
        write_pid_file(t.path(), "caddy", 42).unwrap();
        assert_eq!(read_pid_file(t.path(), "caddy").unwrap(), Some(42));
        remove_pid_file(t.path(), "caddy");
        assert_eq!(read_pid_file(t.path(), "caddy").unwrap(), None);
    }
}
