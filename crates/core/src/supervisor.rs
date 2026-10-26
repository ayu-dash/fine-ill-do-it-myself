use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::proc::{read_pid_file, remove_pid_file, write_pid_file, Runner};
use crate::service::{ServiceState, ServiceStatus};

/// Everything a service needs that has been resolved from disk.
#[derive(Debug, Clone)]
pub struct Env {
    pub paths: Paths,
    pub manifest_dir: PathBuf,
}

impl Env {
    /// Load from the real system locations (`$XDG_DATA_HOME`, `$HOME`).
    /// Used by the binary crate; tests construct [`Env::new`] with temp dirs.
    pub fn load_real() -> Result<Self> {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| Error::Other("cannot resolve home (set $HOME)".into()))?;
        let data = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".local/share"));
        Ok(Env::new(
            crate::paths::default_root(data),
            crate::paths::default_sites(home),
        ))
    }
    pub fn new(root: PathBuf, sites: PathBuf) -> Self {
        let paths = Paths::new(root, sites);
        let manifest_dir = paths.root().to_path_buf();
        Env {
            paths,
            manifest_dir,
        }
    }
    pub fn manifest_path(&self) -> PathBuf {
        self.manifest_dir.join("sources.toml")
    }
    pub fn lock_path(&self) -> PathBuf {
        self.paths.root.join("lock.toml")
    }
    pub fn caddyfile_path(&self) -> PathBuf {
        self.paths.etc().join("Caddyfile")
    }
    pub fn caddy_bin(&self) -> PathBuf {
        self.paths.bin().join("caddy")
    }
    pub fn caddy_pid(&self) -> PathBuf {
        self.paths.pids().join("caddy.pid")
    }
    pub fn caddy_log(&self) -> PathBuf {
        self.paths.logs().join("caddy.log")
    }
    pub fn fpm_conf_path(&self, version: &str) -> PathBuf {
        self.paths.etc().join(format!("php-{version}-fpm.conf"))
    }
    pub fn fpm_bin(&self, version: &str) -> PathBuf {
        self.paths.bin().join(format!("php-fpm-{version}"))
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        self.paths.create_all()?;
        Ok(())
    }
}

/// Which HTTP ports to bind. 80/443 are defaults when available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortPlan {
    pub http: u16,
    pub https: u16,
}

impl Default for PortPlan {
    fn default() -> Self {
        Self {
            http: 80,
            https: 443,
        }
    }
}

/// Supervisor: starts/stops services as detached background processes with
/// PID files. `Runner` and init-directory roots are injected so tests use fakes.
#[derive(Debug, Clone)]
pub struct Supervisor<R: Runner> {
    pub env: Env,
    pub ports: PortPlan,
    pub runner: R,
}

impl<R: Runner> Supervisor<R> {
    pub fn start_multi(&self, cmds: Vec<Vec<String>>, _name: &str) -> Result<Vec<u32>> {
        let mut pids = Vec::new();
        for cmd in cmds {
            pids.push(self.runner.spawn(&cmd)?);
        }
        Ok(pids)
    }

    pub fn write_pid(&self, name: &str, pid: u32) -> Result<()> {
        write_pid_file(&self.env.paths.pids(), name, pid)?;
        Ok(())
    }

    pub fn read_pid(&self, name: &str) -> Result<Option<u32>> {
        Ok(read_pid_file(&self.env.paths.pids(), name)?)
    }

    pub fn stop(&self, name: &str) -> Result<()> {
        if let Some(pid) = read_pid_file(&self.env.paths.pids(), name)? {
            self.runner.kill(pid)?;
            remove_pid_file(&self.env.paths.pids(), name);
        }
        Ok(())
    }

    pub fn status(&self, name: &str) -> ServiceStatus {
        let pid = read_pid_file(&self.env.paths.pids(), name).ok().flatten();
        let alive = pid.filter(|p| self.runner.is_running(*p));
        let _ = &self.ports;
        ServiceStatus {
            name: name.into(),
            state: if alive.is_some() {
                ServiceState::Running
            } else {
                ServiceState::Stopped
            },
            pid: alive,
        }
    }

    /// Parse a list of `*.pid` files present in the pid dir.
    pub fn known_services(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.env.paths.pids()) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if let Some(base) = n.strip_suffix(".pid") {
                    v.push(base.to_string());
                }
            }
        }
        v.sort();
        v
    }
}

/// Given a command, wrap it so output is appended to the service log file and
/// the shell is replaced by the service binary (consistent pid).
pub fn with_log_append(cmd: &[String], log: &Path) -> Result<Vec<String>> {
    if cmd.is_empty() {
        return Err(Error::Other("empty command".into()));
    }
    let joined = cmd.join(" ");
    let quoted = shlex_quote(&joined);
    Ok(vec![
        "/bin/sh".into(),
        "-c".into(),
        format!("exec {} >> {} 2>&1", quoted, log.display()),
    ])
}

/// Single-quote a string so that `sh` treats it as one word.
pub fn shlex_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRunner {
        spawned: Mutex<Vec<Vec<String>>>,
        pid: Mutex<u32>,
        alive: Mutex<Vec<u32>>,
    }

    impl Runner for FakeRunner {
        fn spawn(&self, cmd: &[String]) -> std::result::Result<u32, std::io::Error> {
            self.spawned.lock().unwrap().push(cmd.to_vec());
            let mut pid = self.pid.lock().unwrap();
            *pid += 1;
            self.alive.lock().unwrap().push(*pid);
            Ok(*pid)
        }
        fn is_running(&self, pid: u32) -> bool {
            self.alive.lock().unwrap().contains(&pid)
        }
        fn kill(&self, pid: u32) -> std::result::Result<(), std::io::Error> {
            self.alive.lock().unwrap().retain(|&p| p != pid);
            Ok(())
        }
    }

    fn sup() -> Supervisor<FakeRunner> {
        let t = tempfile::tempdir().unwrap();
        Supervisor {
            env: Env::new(t.path().join("app"), t.path().join("Sites")),
            ports: PortPlan {
                http: 80,
                https: 443,
            },
            runner: FakeRunner::default(),
        }
    }

    #[test]
    fn start_writes_pid_and_reports_running() {
        let s = sup();
        s.env.ensure_dirs().ok();
        let pid = s
            .start_multi(vec![vec!["caddy".to_string()]], "caddy")
            .unwrap();
        s.env.ensure_dirs().unwrap();
        s.write_pid("caddy", pid[0]).unwrap();
        assert_eq!(s.status("caddy").state, ServiceState::Running);
    }

    #[test]
    fn stop_kills_and_clears_pid() {
        let s = sup();
        s.env.ensure_dirs().ok();
        let pid = s.start_multi(vec![vec!["x".to_string()]], "x").unwrap();
        s.env.ensure_dirs().unwrap();
        s.write_pid("x", pid[0]).unwrap();
        s.stop("x").unwrap();
        assert_eq!(s.status("x").state, ServiceState::Stopped);
        assert_eq!(s.read_pid("x").unwrap(), None);
    }

    #[test]
    fn with_log_appends_wraps_command() {
        let cmd = vec!["caddy".to_string(), "run".to_string()];
        let wrapped = with_log_append(&cmd, Path::new("/tmp/caddy.log")).unwrap();
        assert_eq!(wrapped[0], "/bin/sh");
        assert!(wrapped[2].contains(">> /tmp/caddy.log 2>&1"));
    }
}
