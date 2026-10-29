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
    /// Load the environment from an explicit override or the user's XDG data directory.
    pub fn load_real() -> Result<Self> {
        if let Ok(dir) = std::env::var("FIDIM_DATA_ROOT") {
            let root = PathBuf::from(dir);
            let projects = crate::paths::default_projects(&root);
            return Ok(Env::new(root, projects));
        }
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| Error::Other("cannot resolve home (set $HOME)".into()))?;
        let data = std::env::var("XDG_DATA_HOME")
            .ok()
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        let root = crate::paths::default_root(data);
        Ok(Env::new(
            root.clone(),
            crate::paths::default_projects(&root),
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
        let nested = self.paths.bin().join("caddy").join("caddy");
        if nested.exists() {
            return nested;
        }
        let flat = self.paths.bin().join("caddy");
        if flat.is_file() {
            return flat;
        }
        if self.paths.bin().join("caddy").is_dir() {
            return nested;
        }
        flat
    }
    pub fn caddy_pid(&self) -> PathBuf {
        self.paths.pids().join("caddy.pid")
    }
    pub fn caddy_log(&self) -> PathBuf {
        self.paths.logs().join("caddy.log")
    }
    pub fn php_etc_dir(&self, version: &str) -> PathBuf {
        self.paths.etc().join("php").join(version)
    }
    pub fn fpm_conf_path(&self, version: &str) -> PathBuf {
        self.php_etc_dir(version).join("fpm.conf")
    }
    pub fn php_ini_path(&self, version: &str) -> PathBuf {
        self.php_etc_dir(version).join("php.ini")
    }
    pub fn fpm_bin(&self, version: &str) -> PathBuf {
        let nested = self
            .paths
            .bin()
            .join("php")
            .join(format!("php-fpm-{version}"));
        if nested.exists() {
            return nested;
        }
        let flat = self.paths.bin().join(format!("php-fpm-{version}"));
        if flat.is_file() {
            return flat;
        }
        if self.paths.bin().join("php").is_dir() {
            return nested;
        }
        flat
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
    pub fpm_base: u16,
    pub mariadb: u16,
    /// per-web-server port overrides (caddy, httpd, nginx, lighttpd)
    pub caddy_http: u16,
    pub caddy_https: u16,
    pub httpd_http: u16,
    pub httpd_https: u16,
    pub nginx_http: u16,
    pub nginx_https: u16,
    pub lighttpd_http: u16,
    pub lighttpd_https: u16,
}

impl Default for PortPlan {
    fn default() -> Self {
        Self {
            http: 80,
            https: 443,
            fpm_base: 9000,
            mariadb: 3306,
            caddy_http: 8080,
            caddy_https: 8443,
            httpd_http: 8081,
            httpd_https: 8444,
            nginx_http: 8082,
            nginx_https: 8445,
            lighttpd_http: 8083,
            lighttpd_https: 8446,
        }
    }
}

impl PortPlan {
    /// http/https pair belonging to one web server's own port config
    /// (caddy_http for caddy, httpd_http for apache, ...); unknown names
    /// fall back to the shared global pair.
    pub fn web_ports(&self, web: &str) -> (u16, u16) {
        match web {
            crate::orchestrator::CADDY => (self.caddy_http, self.caddy_https),
            crate::orchestrator::APACHE => (self.httpd_http, self.httpd_https),
            crate::orchestrator::NGINX => (self.nginx_http, self.nginx_https),
            crate::orchestrator::LIGHTTPD => (self.lighttpd_http, self.lighttpd_https),
            _ => (self.http, self.https),
        }
    }

    /// Resolve ports: `<etc>/ports.toml < env vars > defaults`.
    /// `env` maps a variable name (FIDIM_HTTP_PORT, ...) to its numeric value.
    pub fn resolve(etc: &std::path::Path, env: impl Fn(&str) -> Option<String>) -> Self {
        let text = std::fs::read_to_string(etc.join("ports.toml")).unwrap_or_default();
        let mut plan = Self::parse(&text);
        if let Some(v) = env("FIDIM_HTTP_PORT").and_then(|v| v.parse().ok()) {
            plan.http = v;
        }
        if let Some(v) = env("FIDIM_HTTPS_PORT").and_then(|v| v.parse().ok()) {
            plan.https = v;
        }
        if let Some(v) = env("FIDIM_FPM_BASE").and_then(|v| v.parse().ok()) {
            plan.fpm_base = v;
        }
        if let Some(v) = env("FIDIM_MARIADB_PORT").and_then(|v| v.parse().ok()) {
            plan.mariadb = v;
        }
        plan
    }

    /// Parse a `http = 8080` style ports.toml (missing keys keep defaults).
    pub fn parse(text: &str) -> Self {
        let mut p = Self::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim();
                let val = v.trim().trim_matches('"');
                if let Ok(n) = val.parse::<u16>() {
                    match key {
                        "http" => p.http = n,
                        "https" => p.https = n,
                        "fpm_base" => p.fpm_base = n,
                        "mariadb" => p.mariadb = n,
                        "caddy_http" => p.caddy_http = n,
                        "caddy_https" => p.caddy_https = n,
                        "httpd_http" => p.httpd_http = n,
                        "httpd_https" => p.httpd_https = n,
                        "nginx_http" => p.nginx_http = n,
                        "nginx_https" => p.nginx_https = n,
                        "lighttpd_http" => p.lighttpd_http = n,
                        "lighttpd_https" => p.lighttpd_https = n,
                        _ => {}
                    }
                }
            }
        }
        p
    }
}

/// Look up the pids listening on a TCP port. Injected so tests stay off real
/// sockets; production passes [`crate::proc::listening_pids`].
pub type PortScan<'a> = &'a dyn Fn(u16) -> std::io::Result<Vec<u32>>;

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

    /// Port whose listener should be scanned for orphan workers when a
    /// service has no (or a stale) pid file. Web servers scan *their own*
    /// http port, never the shared global one.
    pub fn orphan_reaping_port(&self, name: &str) -> Option<u16> {
        if name == "mariadb" {
            Some(self.ports.mariadb)
        } else if [
            crate::orchestrator::CADDY,
            crate::orchestrator::APACHE,
            crate::orchestrator::NGINX,
            crate::orchestrator::LIGHTTPD,
        ]
        .contains(&name)
        {
            Some(self.ports.web_ports(name).0)
        } else {
            name.strip_prefix("php-")
                .map(|v| crate::php::fpm::fpm_port_for(v, 9000, &self.ports))
        }
    }

    /// Kill processes still listening on this service's port (php-fpm workers
    /// outlive their master) and drop a pid file whose process is gone.
    /// The port lookup is injected so tests never touch real sockets.
    pub fn reap_orphans(&self, name: &str, scan: PortScan<'_>) -> Result<()> {
        if let Some(pid) = read_pid_file(&self.env.paths.pids(), name)? {
            if !self.runner.is_running(pid) {
                remove_pid_file(&self.env.paths.pids(), name);
            }
        }
        // Only php-fpm (workers outlive the master) and mariadbd (socket
        // helpers) fork detached children. A web server that died leaves
        // nothing behind, and its port may belong to a system server we must
        // never touch.
        if !forks_detached_children(name) {
            return Ok(());
        }
        let Some(port) = self.orphan_reaping_port(name).filter(|p| *p != 0) else {
            return Ok(());
        };
        let Ok(pids) = scan(port) else {
            return Ok(());
        };
        // For mariadb only touch processes started with OUR my.cnf: a system
        // MySQL/MariaDB that happens to listen on the same port must survive.
        let our_cnf = self.env.paths.etc().join("my.cnf");
        for pid in pids {
            let ours = name != crate::orchestrator::MARIADB
                || crate::proc::pid_cmdline_has(pid, our_cnf.as_os_str());
            if ours {
                let _ = self.runner.kill(pid);
            }
        }
        Ok(())
    }

    pub fn stop(&self, name: &str) -> Result<()> {
        if let Some(pid) = read_pid_file(&self.env.paths.pids(), name)? {
            self.runner.kill(pid)?;
            remove_pid_file(&self.env.paths.pids(), name);
        }
        self.reap_orphans(name, &crate::proc::listening_pids)
    }

    pub fn status(&self, name: &str) -> ServiceStatus {
        let pid = read_pid_file(&self.env.paths.pids(), name).ok().flatten();
        let alive = pid.filter(|p| self.runner.is_running(*p));
        let _ = &self.ports;
        let state = if alive.is_some() {
            ServiceState::Running
        } else if pid.is_some() {
            // we launched it at some point but the process is gone
            ServiceState::Error("process exited; see logs".into())
        } else {
            let is_lang = crate::lang::SUPPORTED_LANGUAGES
                .iter()
                .any(|&l| name == l || name.starts_with(&format!("{l}-")));
            if is_lang && !name.starts_with("php") {
                let base = name.split('-').next().unwrap_or(name);
                if crate::lang::active_lang_version(&self.env, base).is_some() {
                    ServiceState::Running
                } else {
                    ServiceState::Stopped
                }
            } else {
                ServiceState::Stopped
            }
        };
        ServiceStatus {
            name: name.into(),
            state,
            pid: alive,
        }
    }

    /// Parse a list of `*.pid` files present in the pid dir. Pid files that
    /// mariadbd writes next to its socket (`db.sock.pid` on older installs) are
    /// not services and are skipped.
    pub fn known_services(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.env.paths.pids()) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if let Some(base) = n.strip_suffix(".pid") {
                    if base.ends_with(".sock") {
                        continue;
                    }
                    v.push(base.to_string());
                }
            }
        }
        v.sort();
        v
    }
}

/// True for services that spawn detached children which survive their master.
fn forks_detached_children(name: &str) -> bool {
    name.starts_with("php-") || name == crate::orchestrator::MARIADB
}

/// Given a command, wrap it so output is appended to the service log file and
/// the shell is replaced by the service binary (consistent pid).
pub fn with_log_append(cmd: &[String], log: &Path) -> Result<Vec<String>> {
    if cmd.is_empty() {
        return Err(Error::Other("empty command".into()));
    }
    let quoted = cmd
        .iter()
        .map(|a| shlex_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
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
            ports: PortPlan::default(),
            runner: FakeRunner::default(),
        }
    }

    #[test]
    fn known_services_ignores_the_mariadb_socket_pid_file() {
        let t = tempfile::tempdir().unwrap();
        let s = Supervisor {
            env: Env::new(t.path().join("app"), t.path().join("Sites")),
            ports: PortPlan::default(),
            runner: FakeRunner::default(),
        };
        s.env.ensure_dirs().unwrap();
        std::fs::write(s.env.paths.pids().join("mariadb.pid"), "1").unwrap();
        // mariadbd writes <socket>.pid next to the socket; it is not a service
        std::fs::write(s.env.paths.pids().join("db.sock.pid"), "1").unwrap();

        assert_eq!(s.known_services(), vec!["mariadb".to_string()]);
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
    fn web_ports_differ_per_server_and_fall_back_for_unknown() {
        let p = PortPlan::default();
        let mut q = PortPlan {
            nginx_http: 8181,
            nginx_https: 8444,
            ..PortPlan::default()
        };
        q.http = 80;
        q.https = 443;
        assert_eq!(q.web_ports("nginx"), (8181, 8444));
        assert_eq!(q.web_ports("caddy"), (p.caddy_http, p.caddy_https));
        assert_eq!(q.web_ports("httpd"), (p.httpd_http, p.httpd_https));
        assert_eq!(q.web_ports("lighttpd"), (p.lighttpd_http, p.lighttpd_https));
        assert_eq!(q.web_ports("nonsense"), (80, 443));
    }

    #[test]
    fn orphan_reaping_port_is_per_server() {
        let mut s = sup();
        s.ports.nginx_http = 8181;
        assert_eq!(s.orphan_reaping_port("nginx"), Some(8181));
        assert_eq!(s.orphan_reaping_port("caddy"), Some(s.ports.caddy_http));
        assert_eq!(s.orphan_reaping_port("httpd"), Some(s.ports.httpd_http));
        assert_eq!(s.orphan_reaping_port("mariadb"), Some(s.ports.mariadb));
        assert_eq!(s.orphan_reaping_port("php-8.4"), Some(9004));
        assert_eq!(s.orphan_reaping_port("something"), None);
    }

    #[test]
    fn reap_orphans_kills_php_workers_holding_the_fpm_port() {
        let s = sup();
        s.env.ensure_dirs().unwrap();
        s.runner.alive.lock().unwrap().extend([111, 222]);

        s.reap_orphans("php-8.2", &|port| {
            assert_eq!(port, 9002, "must scan the port of that php version");
            Ok(vec![111, 222])
        })
        .unwrap();

        assert!(
            s.runner.alive.lock().unwrap().is_empty(),
            "orphan workers must be killed"
        );
    }

    #[test]
    fn reap_orphans_clears_a_pid_file_whose_process_is_gone() {
        let s = sup();
        s.env.ensure_dirs().unwrap();
        s.write_pid("php-8.2", 4242).unwrap();

        s.reap_orphans("php-8.2", &|_| Ok(vec![])).unwrap();

        assert_eq!(
            s.read_pid("php-8.2").unwrap(),
            None,
            "a stale pid file must not survive"
        );
        assert_eq!(
            s.status("php-8.2").state,
            ServiceState::Stopped,
            "status must not keep reporting Error"
        );
    }

    #[test]
    fn reap_orphans_keeps_our_pid_file_when_the_process_is_alive() {
        let s = sup();
        s.env.ensure_dirs().unwrap();
        s.runner.alive.lock().unwrap().push(77);
        s.write_pid("php-8.2", 77).unwrap();

        s.reap_orphans("php-8.2", &|_| Ok(vec![])).unwrap();

        assert_eq!(s.read_pid("php-8.2").unwrap(), Some(77));
    }

    #[test]
    fn reap_orphans_never_kills_a_foreign_mariadb_on_the_same_port() {
        let s = sup();
        s.env.ensure_dirs().unwrap();
        // pid 111 is not a process of ours: its cmdline lacks our my.cnf
        s.runner.alive.lock().unwrap().push(111);

        s.reap_orphans("mariadb", &|_| Ok(vec![111])).unwrap();

        assert_eq!(
            s.runner.alive.lock().unwrap().as_slice(),
            &[111],
            "a system MariaDB listening on our port must survive"
        );
    }

    #[test]
    fn reap_orphans_does_not_scan_ports_for_web_servers() {
        let s = sup();
        s.env.ensure_dirs().unwrap();
        s.runner.alive.lock().unwrap().push(111);

        s.reap_orphans("caddy", &|_| {
            panic!("web servers do not fork detached children; never scan their port")
        })
        .unwrap();

        assert_eq!(s.runner.alive.lock().unwrap().as_slice(), &[111]);
    }

    #[test]
    fn with_log_appends_wraps_command() {
        let cmd = vec!["caddy".to_string(), "run".to_string()];
        let wrapped = with_log_append(&cmd, Path::new("/tmp/caddy.log")).unwrap();
        assert_eq!(wrapped[0], "/bin/sh");
        assert!(wrapped[2].contains(">> /tmp/caddy.log 2>&1"));
    }
}
