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
        // kill the whole process tree: php-fpm/nginx masters respawn children
        for p in child_pids(pid) {
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(p.to_string())
                .output();
        }
        std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .output()?;
        // grace period: escalate anything left
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
        while std::time::Instant::now() < deadline {
            if RealRunner.is_running(pid) && child_pids(pid).is_empty() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for p in child_pids(pid) {
            let _ = std::process::Command::new("kill")
                .arg("-KILL")
                .arg(p.to_string())
                .output();
        }
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .output();
        Ok(())
    }
}

/// PIDs whose parent is `parent` by reading /proc/<pid>/stat (Linux).
pub fn child_pids(parent: u32) -> Vec<u32> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let name = e.file_name();
            let pid = match name.to_str().and_then(|s| s.parse::<u32>().ok()) {
                Some(p) if p != parent => p,
                _ => continue,
            };
            if let Ok(stat) = std::fs::read(e.path().join("stat")) {
                if let Ok(txt) = String::from_utf8(stat) {
                    // comm may contain spaces/parens: take after last ')'
                    if let Some(idx) = txt.rfind(')') {
                        let rest = &txt[idx + 1..];
                        let fields: Vec<&str> = rest.split_whitespace().collect();
                        // after ')' the offsets are already shifted by state
                        let (state, ppid) =
                            { (fields[0], fields.get(1).and_then(|s| s.parse::<u32>().ok())) };
                        let _ = state;
                        if ppid == Some(parent) {
                            out.push(pid);
                        }
                    }
                }
            }
        }
    }
    out
}

/// PIDs of processes LISTENing on `port` (TCP, all interfaces) by matching
/// /proc/net/tcp inodes against /proc/<pid>/fd entries.
pub fn listening_pids(port: u16) -> std::result::Result<Vec<u32>, std::io::Error> {
    let hex_port = format!("{:04X}", port);
    let mut inodes: Vec<String> = Vec::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(table) {
            for line in text.lines().skip(1) {
                let col: Vec<&str> = line.split_whitespace().collect();
                if col.len() >= 10 && col[1].ends_with(&hex_port) && col[3] == "0A" {
                    inodes.push(col[9].to_string());
                }
            }
        }
    }
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let pid = match e.file_name().to_str().and_then(|p| p.parse::<u32>().ok()) {
                Some(p) => p,
                None => continue,
            };
            let fd_dir = e.path().join("fd");
            if let Ok(fds) = std::fs::read_dir(&fd_dir) {
                for fd in fds.flatten() {
                    if let Ok(target) = std::fs::read_link(fd.path()) {
                        let t = target.to_string_lossy().to_string();
                        if let Some(inode) =
                            t.strip_prefix("socket:[").and_then(|x| x.strip_suffix(']'))
                        {
                            if inodes.iter().any(|i| i == inode) {
                                out.push(pid);
                            }
                        }
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Does a NUL-separated `/proc/<pid>/cmdline` buffer contain `arg` verbatim?
/// Used to distinguish fidim-spawned processes from unrelated ones that happen
/// to listen on the same port.
pub fn cmdline_has(raw: &[u8], arg: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let needle = arg.as_bytes();
    !needle.is_empty() && raw.split(|b| *b == 0).any(|part| part == needle)
}

/// Read `/proc/<pid>/cmdline` and match one argument; false when unreadable.
pub fn pid_cmdline_has(pid: u32, arg: &std::ffi::OsStr) -> bool {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|raw| cmdline_has(&raw, arg))
        .unwrap_or(false)
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

    #[test]
    fn cmdline_matches_only_the_exact_argument() {
        use std::ffi::OsStr;
        let ours = b"/opt/mariadb/bin/mariadbd\0--defaults-file\0/app/etc/my.cnf\0";
        let system = b"/usr/sbin/mysqld\0--defaults-file\0/etc/mysql/my.cnf\0";
        let cnf = OsStr::new("/app/etc/my.cnf");
        assert!(cmdline_has(ours, cnf));
        assert!(!cmdline_has(system, cnf));
        assert!(!cmdline_has(b"", cnf));
    }
}
