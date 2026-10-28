use std::os::unix::process::CommandExt;
use std::path::Path;

/// Process control abstraction so management logic can be tested without
/// spawning real processes (real impl = std::process).
pub trait Runner: Send + Sync {
    fn spawn(&self, cmd: &[String]) -> Result<u32, std::io::Error>;
    fn is_running(&self, pid: u32) -> bool;
    fn kill(&self, pid: u32) -> std::result::Result<(), std::io::Error>;
    /// True when `pid` or one of its descendants owns a listening socket on
    /// `port`. Fakes default to true so unit tests never touch real sockets;
    /// the real runner verifies the bind actually happened.
    fn owns_port(&self, _pid: u32, _port: u16) -> bool {
        true
    }
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
        if !std::path::Path::new(format!("/proc/{pid}").as_str()).exists() {
            return false;
        }
        // An exited child stays in /proc as a zombie until reaped; it is not
        // a running service (a failed bind leaves exactly this behind).
        proc_state(pid).map(|s| s != 'Z').unwrap_or(true)
    }
    fn owns_port(&self, pid: u32, port: u16) -> bool {
        let Ok(pids) = listening_pids(port) else {
            return false;
        };
        pids.into_iter()
            .any(|listener| listener == pid || is_descendant(listener, pid))
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
            if !RealRunner.is_running(pid) {
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

/// Parent pid from `/proc/<pid>/stat` (field 4 after the command name).
fn parent_pid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Walk the parent chain: is `pid` the same process as `ancestor` or below it?
fn is_descendant(pid: u32, ancestor: u32) -> bool {
    let mut current = pid;
    for _ in 0..64 {
        if current == ancestor {
            return true;
        }
        match parent_pid(current) {
            Some(parent) if parent > 1 => current = parent,
            _ => return false,
        }
    }
    false
}

/// State letter from `/proc/<pid>/stat` (`R`, `S`, `Z`, ...).
fn proc_state(pid: u32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().next()?.chars().next()
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

/// Does a NUL-separated `/proc/<pid>/cmdline` buffer contain `arg` verbatim or
/// as the value of a `--flag=<arg>` argument? The latter is how mariadbd
/// receives `--defaults-file=<path>`. Used to distinguish fidim-spawned
/// processes from unrelated ones listening on the same port.
pub fn cmdline_has(raw: &[u8], arg: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let needle = arg.as_bytes();
    if needle.is_empty() {
        return false;
    }
    raw.split(|b| *b == 0).any(|part| {
        part == needle
            || (part.starts_with(b"--")
                && part.ends_with(needle)
                && part.len() > needle.len()
                && part[part.len() - needle.len() - 1] == b'=')
    })
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

/// Run a command to completion and capture its output. Composer and the
/// MariaDB client are synchronous tools, unlike the detached services driven
/// through [`Runner`]; tests inject a fake so no process is spawned.
pub trait CommandRunner: Send + Sync {
    fn run(&self, cmd: &[String], envs: &[(String, String)]) -> crate::error::Result<String>;
}

#[derive(Debug, Clone, Default)]
pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn run(&self, cmd: &[String], envs: &[(String, String)]) -> crate::error::Result<String> {
        let Some(program) = cmd.first() else {
            return Err(crate::error::Error::Other("empty command".into()));
        };
        let out = std::process::Command::new(program)
            .args(&cmd[1..])
            .envs(envs.iter().map(|(k, v)| (k, v)))
            .output()
            .map_err(|e| crate::error::Error::Other(format!("run {program}: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let detail = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            return Err(crate::error::Error::Other(format!(
                "{program} exited with {}: {detail}",
                out.status.code().unwrap_or(-1)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_runner_kill_escalates_when_sigterm_is_ignored() {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("trap '' TERM; exec sleep 5")
            .spawn()
            .unwrap();
        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(100));

        RealRunner.kill(pid).unwrap();

        // SIGKILL is delivered asynchronously; give the kernel a moment.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while RealRunner.is_running(pid) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !RealRunner.is_running(pid),
            "kill must escalate to SIGKILL when SIGTERM is ignored"
        );
        let _ = child.wait();
    }

    #[test]
    fn real_runner_does_not_report_an_exited_child_as_running() {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .unwrap();
        let pid = child.id();
        let stat = format!("/proc/{pid}/stat");
        for _ in 0..50 {
            if std::fs::read_to_string(&stat)
                .map(|s| s.contains(") Z "))
                .unwrap_or(false)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        assert!(
            !RealRunner.is_running(pid),
            "a zombie child must not be reported as running"
        );
        let _ = child.wait();
    }

    #[test]
    fn pid_file_round_trip() {
        let t = tempfile::tempdir().unwrap();
        write_pid_file(t.path(), "caddy", 42).unwrap();
        assert_eq!(read_pid_file(t.path(), "caddy").unwrap(), Some(42));
        remove_pid_file(t.path(), "caddy");
        assert_eq!(read_pid_file(t.path(), "caddy").unwrap(), None);
    }

    #[test]
    fn cmdline_matches_the_argument_or_an_equals_flag() {
        use std::ffi::OsStr;
        let ours = b"/opt/mariadb/bin/mariadbd\0--defaults-file\0/app/etc/my.cnf\0";
        let joined = b"/opt/mariadb/bin/mariadbd\0--defaults-file=/app/etc/my.cnf\0";
        let system = b"/usr/sbin/mysqld\0--defaults-file\0/etc/mysql/my.cnf\0";
        let cnf = OsStr::new("/app/etc/my.cnf");
        assert!(cmdline_has(ours, cnf));
        assert!(
            cmdline_has(joined, cnf),
            "mariadbd receives --defaults-file=<path> as one argument"
        );
        assert!(!cmdline_has(system, cnf));
        assert!(!cmdline_has(b"", cnf));
    }

    #[test]
    fn real_command_runner_returns_stdout() {
        let out = RealCommandRunner
            .run(&["/bin/sh".into(), "-c".into(), "printf hello".into()], &[])
            .unwrap();
        assert_eq!(out, "hello");
    }

    #[test]
    fn real_command_runner_passes_extra_environment() {
        let out = RealCommandRunner
            .run(
                &[
                    "/bin/sh".into(),
                    "-c".into(),
                    "printf %s \"$FIDIM_TEST_VAR\"".into(),
                ],
                &[("FIDIM_TEST_VAR".into(), "works".into())],
            )
            .unwrap();
        assert_eq!(out, "works");
    }

    #[test]
    fn real_command_runner_reports_a_non_zero_exit() {
        let err = RealCommandRunner
            .run(
                &[
                    "/bin/sh".into(),
                    "-c".into(),
                    "echo boom >&2; exit 3".into(),
                ],
                &[],
            )
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("boom"), "{msg}");
        assert!(msg.contains('3'), "{msg}");
    }
}
