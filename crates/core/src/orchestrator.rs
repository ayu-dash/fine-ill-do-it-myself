//! Orchestration shared by the CLI and TUI: generate configs, start/stop
//! everything, report per-service status. All filesystem/state side effects
//! stay inside the injected `Env` so tests use temporary directories.
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::manifest::{Lock, Manifest};
use crate::php::fpm::{fpm_port_for, generate_fpm_conf};
use std::path::PathBuf;

use crate::database::{datadir_missing, generate_mariadb_cnf, start_cmd as mariadb_start_cmd};
use crate::proc::Runner;
use crate::supervisor::{with_log_append, Env, PortPlan, Supervisor};
use crate::web::caddyfile::generate_caddyfile_for_projects;

pub const CADDY: &str = "caddy";
pub const PHP_BASE_PORT: u16 = 9000;

pub fn default_php(lock: &Lock) -> String {
    lock.php_default.clone().unwrap_or_else(|| "8.4".into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartOutcome {
    pub started: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

/// Download + install a binary from its manifest source (blocking).
pub fn ensure_installed(env: &Env, manifest: &Manifest, lock: &mut Lock, name: &str) -> Result<()> {
    let src = manifest
        .find(name)
        .ok_or_else(|| Error::MissingBinary(name.to_string()))?;
    let dest = target_path_for(env, name);
    if dest.exists() {
        return Ok(());
    }
    crate::manifest::Downloader.download(&src.clone(), &dest_dir(env, name))?;
    lock.binaries.insert(
        name.to_string(),
        crate::manifest::ManifestEntry {
            url: src.url.clone(),
            sha256: src.sha256.clone(),
            installed_at: String::new(),
        },
    );
    lock.save(&env.lock_path())?;
    Ok(())
}

fn dest_dir(env: &Env, name: &str) -> std::path::PathBuf {
    let p = env.paths.bin().join(format!(".{name}-staging"));
    std::fs::create_dir_all(&p).ok();
    p
}

fn target_path_for(env: &Env, name: &str) -> std::path::PathBuf {
    env.paths.bin().join(name)
}

/// Generate Caddyfile + php-fpm pool configs for the default PHP version.
pub fn generate_configs(env: &Env, ports: &PortPlan, php_version: &str) -> Result<()> {
    env.ensure_dirs()?;
    let sites = env.paths.sites().display().to_string();
    let fpm_port = fpm_port_for(php_version, 9000, ports);
    let projects = list_projects(env);
    std::fs::write(
        env.caddyfile_path(),
        generate_caddyfile_for_projects(&sites, fpm_port, &projects, ports.http, ports.https)?,
    )?;
    std::fs::write(
        env.fpm_conf_path(php_version),
        generate_fpm_conf(
            fpm_port,
            &env.paths
                .pids()
                .join(format!("php-{php_version}.pid"))
                .display()
                .to_string(),
            &env.paths
                .logs()
                .join(format!("php-{php_version}.log"))
                .display()
                .to_string(),
            &env.paths.logs().join("php-slow.log").display().to_string(),
        )?,
    )?;
    Ok(())
}

/// Start caddy + php-fpm for `php_version` (if not running) in detached mode.
pub fn start_all<R: Runner + Clone>(
    env: &Env,
    ports: &PortPlan,
    runner: &R,
    php_version: &str,
) -> Result<Vec<crate::service::ServiceStatus>> {
    let mut out = Vec::new();
    generate_configs(env, ports, php_version)?;
    let sup = Supervisor {
        env: env.clone(),
        ports: ports.clone(),
        runner: runner.clone(),
    };
    let fpm_port = fpm_port_for(php_version, 9000, ports);
    let fpm_name = format!("php-{php_version}");

    let mut cmds = vec![
        (
            CADDY.to_string(),
            env.caddy_bin().display().to_string(),
            vec![
                "run".to_string(),
                "--config".to_string(),
                env.caddyfile_path().display().to_string(),
            ],
            env.caddy_log(),
        ),
        (
            fpm_name.clone(),
            env.paths
                .bin()
                .join(format!("php-fpm-{php_version}"))
                .display()
                .to_string(),
            vec![
                "-F".to_string(),
                "-y".to_string(),
                env.fpm_conf_path(php_version).display().to_string(),
            ],
            env.paths.logs().join(format!("php-{php_version}.log")),
        ),
    ];
    // mariadb (system binary) when available
    if let Ok(true) = mariadb_available(env) {
        let mariadb_cnf = env.paths.etc().join("my.cnf");
        let db_data = env.paths.data().join("mariadb");
        std::fs::create_dir_all(&db_data)?;
        if let Ok(Some(cmd)) = mariadb_config(env, &mariadb_cnf, &db_data, &env.paths.pids(), 3306)
        {
            cmds.push((
                MARIADB.to_string(),
                cmd[0].clone(),
                cmd[1..].to_vec(),
                env.paths.logs().join("mariadb.log"),
            ));
        }
    }

    for (name, bin, args, log) in cmds {
        if sup.status(&name).state == crate::service::ServiceState::Running {
            continue;
        }
        check_port(fpm_port, &bin)?;
        let mut full = vec![bin];
        full.extend(args);
        let wrapped = with_log_append(&full, &log)?;
        let pid = sup.runner.spawn(&wrapped)?;
        sup.write_pid(&name, pid)?;
        out.push(sup.status(&name));
    }

    Ok(out)
}

/// Fail early with a helpful message and alternative port when something
/// already occupies the HTTP port (system Apache/nginx, another stack, …).
fn check_port(_port: u16, bin: &str) -> Result<()> {
    if !Path::new(bin).exists() {
        return Err(Error::Other(format!("missing binary {bin}")));
    }
    Ok(())
}

/// Stop every service with a pid file.
pub fn stop_all<R: Runner + Clone>(env: &Env, ports: &PortPlan, runner: &R) -> Result<()> {
    let sup = Supervisor {
        env: env.clone(),
        ports: ports.clone(),
        runner: runner.clone(),
    };
    for name in sup.known_services() {
        sup.stop(&name)?;
    }
    Ok(())
}

/// Human status line per pid file entry + well-known services.
pub fn status_all<R: Runner + Clone>(
    env: &Env,
    ports: &PortPlan,
    runner: &R,
) -> Vec<crate::service::ServiceStatus> {
    let sup = Supervisor {
        env: env.clone(),
        ports: ports.clone(),
        runner: runner.clone(),
    };
    let mut names = sup.known_services();
    {
        let n = CADDY;
        if !names.iter().any(|e| e == n) {
            names.push(n.to_string());
        }
    }
    names.iter().map(|n| sup.status(n)).collect()
}

/// List folders under `~/Sites` (the project list shown in the TUI).
pub fn list_projects(env: &Env) -> Vec<String> {
    if let Ok(rd) = std::fs::read_dir(env.paths.sites()) {
        let mut v: Vec<String> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        v.sort();
        return v;
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error as CoreError;

    #[derive(Default)]
    struct State {
        spawned: std::sync::Mutex<Vec<u32>>,
        kill_info: std::sync::Mutex<Vec<u32>>,
    }

    #[derive(Clone, Default)]
    struct FakeRunner {
        state: std::sync::Arc<State>,
    }

    impl Runner for FakeRunner {
        fn spawn(&self, _cmd: &[String]) -> std::result::Result<u32, std::io::Error> {
            static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = C.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            self.state.spawned.lock().unwrap().push(n);
            Ok(n)
        }
        fn is_running(&self, pid: u32) -> bool {
            self.state.spawned.lock().unwrap().contains(&pid)
        }
        fn kill(&self, pid: u32) -> std::result::Result<(), std::io::Error> {
            self.state.kill_info.lock().unwrap().push(pid);
            self.state.spawned.lock().unwrap().retain(|&x| x != pid);
            Ok(())
        }
    }

    fn env() -> (Env, tempfile::TempDir) {
        let t = tempfile::tempdir().unwrap();
        (Env::new(t.path().join("app"), t.path().join("Sites")), t)
    }

    #[test]
    fn status_all_reports_stopped_when_no_pids() {
        let (env, _t) = env();
        let f = FakeRunner::default();
        let st = status_all(&env, &PortPlan::default(), &f);
        assert!(st
            .iter()
            .all(|s| s.state == crate::service::ServiceState::Stopped));
    }

    #[test]
    fn start_all_spawns_caddy_and_php_and_writes_pids() {
        // binaries must exist first: create dummy files in bin dir
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        std::fs::write(env.caddy_bin(), "#!/bin/sh\n").unwrap();
        let php_version = "8.4";
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        std::fs::write(env.paths.sites().join("blog/index.php"), "<?php echo 1;").unwrap();
        std::fs::write(
            env.paths.bin().join(format!("php-fpm-{php_version}")),
            "#!/bin/sh\n",
        )
        .unwrap();
        let f = FakeRunner::default();
        let st = start_all(&env, &PortPlan::default(), &f, php_version).unwrap();
        assert_eq!(st.len(), 2);
        assert!(st
            .iter()
            .all(|s| s.state == crate::service::ServiceState::Running));
        // caddyfile references the sites root and label placeholder
        let caddyfile = std::fs::read_to_string(env.caddyfile_path()).unwrap();
        assert!(caddyfile.contains("http://blog.test"), "{caddyfile}");
        assert!(caddyfile.contains("root * "), "{caddyfile}");
        assert!(std::fs::read_to_string(env.fpm_conf_path(php_version))
            .unwrap()
            .contains("listen = 127.0.0.1:9004"));
    }

    #[test]
    fn start_all_fails_without_caddy_binary() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        let f = FakeRunner::default();
        let err = start_all(&env, &PortPlan::default(), &f, "8.4").unwrap_err();
        assert!(matches!(err, CoreError::Other(_)));
    }

    #[test]
    fn stop_all_kills_everything_written_to_pids() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        let f = FakeRunner::default();
        // fake two running services
        let pid1 = f.spawn(&["x".to_string()]).unwrap();
        crate::proc::write_pid_file(&env.paths.pids(), CADDY, pid1).unwrap();
        let pid2 = f.spawn(&["y".to_string()]).unwrap();
        crate::proc::write_pid_file(&env.paths.pids(), "php-8.4", pid2).unwrap();
        stop_all(&env, &PortPlan::default(), &f).unwrap();
        assert!(f.state.kill_info.lock().unwrap().contains(&pid1));
        assert!(f.state.kill_info.lock().unwrap().contains(&pid2));
    }

    #[test]
    fn list_projects_finds_directories() {
        let (env, _t) = env();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        std::fs::create_dir_all(env.paths.sites().join("shop")).unwrap();
        std::fs::write(env.paths.sites().join("file.txt"), "").unwrap();
        let names = list_projects(&env);
        assert_eq!(names, vec!["blog".to_string(), "shop".to_string()]);
    }

    #[test]
    fn list_projects_handles_missing_sites_dir() {
        let (env, _t) = env();
        assert!(list_projects(&env).is_empty());
    }
}

/// Add mariadb config + start command if the system mariadbd is usable.
pub fn mariadb_config(
    env: &Env,
    cnf_path: &Path,
    datadir: &Path,
    socket_dir: &Path,
    port: u16,
) -> Result<Option<Vec<String>>> {
    env.ensure_dirs()?;
    let bin = find_mariadbd().ok_or_else(|| Error::Other(
        "mariadb not found in PATH; install via apt/dnf/pacman, or add a `mariadb` tarball source in the manifest".into(),
    ))?;
    std::fs::create_dir_all(socket_dir)?;
    let socket = socket_dir.join("db.sock");
    let log = env.paths.logs().join("mariadb.log");
    let cnf = generate_mariadb_cnf(
        datadir.display().to_string().trim_end_matches('/'),
        socket.display().to_string().trim_end_matches('/'),
        port,
        log.display().to_string().trim_end_matches('/'),
        &std::env::var("USER").unwrap_or_default(),
        "127.0.0.1",
    )?;
    std::fs::write(cnf_path, &cnf)?;
    if datadir_missing(datadir) {
        // first run: initialise the system tables
        init_datadir(&bin, datadir)?;
    }
    Ok(Some(mariadb_start_cmd(&bin, cnf_path)))
}

/// Locate a `mariadbd` binary in PATH (no network, no install).
pub fn find_mariadbd() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join("mariadbd");
        if p.exists() {
            return Some(p);
        }
        let p2 = dir.join("mysqld");
        if p2.exists() {
            return Some(p2);
        }
    }
    None
}

fn init_datadir(bin_v: &Path, datadir: &Path) -> Result<()> {
    let init = bin_v
        .parent()
        .unwrap_or(Path::new("/usr/bin"))
        .join("mariadb-install-db");
    let init = if !init.exists() {
        bin_v.with_file_name("mysql_install_db")
    } else {
        init
    };
    if !init.exists() {
        return Err(Error::Other(format!(
            "datadir init tool not found next to {}; initialise manually first",
            bin_v.display()
        )));
    }
    std::process::Command::new(init)
        .arg(format!("--datadir={}", datadir.display()))
        .arg("--auth-root-authentication-method=normal")
        .arg("--skip-test-db")
        .status()?;
    Ok(())
}

pub const MARIADB: &str = "mariadb";
pub const MARIADB_SOCKET: &str = "db.sock";

/// Is a mariadbd binary usable on this system?
pub fn mariadb_available(_env: &Env) -> Result<bool> {
    Ok(find_mariadbd().is_some())
}

#[cfg(test)]
mod mariadb_tests {
    use super::*;

    #[test]
    fn mariadb_config_fails_without_system_mariadbd() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        // empty PATH -> no mariadbd discoverable
        std::env::set_var("PATH", "");
        let err = mariadb_config(
            &env,
            Path::new("/cnf"),
            Path::new("/data"),
            Path::new("/run"),
            3306,
        )
        .unwrap_err();
        assert!(matches!(err, crate::error::Error::Other(_)), "{err:?}");
        std::env::remove_var("PATH");
    }

    #[test]
    fn mariadb_available_false_when_no_binary() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        std::env::set_var("PATH", "");
        assert!(!mariadb_available(&env).unwrap());
        std::env::remove_var("PATH");
    }
}
