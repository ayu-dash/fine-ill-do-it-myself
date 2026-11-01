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
use crate::web::caddyfile::generate_caddyfile_for_sites;

pub const CADDY: &str = "caddy";
pub const NGINX: &str = "nginx";
pub const APACHE: &str = "httpd";
pub const LIGHTTPD: &str = "lighttpd";
pub const PHP_BASE_PORT: u16 = 9000;

pub fn default_php(lock: &Lock) -> String {
    lock.php_default.clone().unwrap_or_else(|| "8.4".into())
}

/// Which supported web server the user picked, falling back to Caddy if the
/// persisted value is invalid.
pub fn default_web(lock: &Lock) -> String {
    lock.web_default
        .as_deref()
        .filter(|web| [CADDY, NGINX, APACHE, LIGHTTPD].contains(web))
        .unwrap_or(CADDY)
        .to_string()
}

/// Load the default web server from lock.toml (caddy when unset).
pub fn web_server(env: &Env) -> String {
    let lock = Lock::load(&env.lock_path()).unwrap_or_default();
    default_web(&lock)
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

pub fn binary_subdir_for(name: &str) -> &'static str {
    if name.starts_with("php") {
        "php"
    } else if name.starts_with("caddy") {
        "caddy"
    } else if name.starts_with("nginx") {
        "nginx"
    } else if name.starts_with("httpd") || name.starts_with("apache") {
        "httpd"
    } else if name.starts_with("lighttpd") {
        "lighttpd"
    } else {
        ""
    }
}

fn dest_dir(env: &Env, name: &str) -> std::path::PathBuf {
    let sub = binary_subdir_for(name);
    let p = if !sub.is_empty() && env.paths.bin().join(sub).is_dir() {
        env.paths.bin().join(sub).join(format!(".{name}-staging"))
    } else {
        env.paths.bin().join(format!(".{name}-staging"))
    };
    std::fs::create_dir_all(&p).ok();
    p
}

fn target_path_for(env: &Env, name: &str) -> std::path::PathBuf {
    let sub = binary_subdir_for(name);
    if !sub.is_empty() {
        let nested = env.paths.bin().join(sub).join(name);
        if nested.exists() || env.paths.bin().join(sub).is_dir() {
            return nested;
        }
    }
    let flat = env.paths.bin().join(name);
    if flat.is_file() {
        return flat;
    }
    if !sub.is_empty() {
        env.paths.bin().join(sub).join(name)
    } else {
        flat
    }
}

/// Generate Caddyfile + php-fpm pool configs for the default PHP version.
pub fn generate_configs(env: &Env, ports: &PortPlan, php_version: &str) -> Result<()> {
    let lock = Lock::load(&env.lock_path()).unwrap_or_default();
    generate_web_configs(env, ports, php_version, &default_web(&lock))
}

/// Generate the vhost/config set of ONE web server (its own http/https pair),
/// the local CA + leaf certs, and the php-fpm pool.
pub fn generate_web_configs(
    env: &Env,
    ports: &PortPlan,
    php_version: &str,
    web: &str,
) -> Result<()> {
    env.ensure_dirs()?;
    let fpm_port = fpm_port_for(php_version, ports.fpm_base, ports);
    // phpMyAdmin is a normal site once installed: link it into Sites before
    // the project list is read so vhosts/certs/hosts pick it up.
    if let Err(e) = crate::phpmyadmin::ensure_site(env, ports) {
        eprintln!("warn: phpMyAdmin site skipped: {e}");
    }
    let sites = list_sites(env);
    let projects: Vec<String> = sites.iter().map(|s| s.name.clone()).collect();
    // local CA + per-project leaf certs (silent no-op without openssl)
    if let Err(e) = crate::web::certs::regenerate(&env.paths.etc(), &projects) {
        eprintln!("warn: certificate generation skipped: {e}");
    }
    let certs_dir = env.paths.etc().join("certs");
    let certs_str = certs_dir.display().to_string();
    // each web server binds its OWN http/https pair, never the shared global one
    let (http_port, https_port) = ports.web_ports(web);
    // Every server gets the same dashboard so `fidim start` is useful before a
    // first project exists (and for unknown hostnames afterwards).
    let dashboard_dir = crate::web::dashboard::write(&env.paths.etc(), &projects, http_port)?;
    let certs_pair = |p: &str| {
        let crt = certs_dir.join(format!("{p}.crt"));
        let key = certs_dir.join(format!("{p}.key"));
        (crt.exists() && key.exists()).then_some((crt, key))
    };
    let web = web.to_string();
    match web.as_str() {
        LIGHTTPD => {
            // A build without OpenSSL ships no mod_openssl.so; advertising TLS
            // then makes lighttpd fail at startup, so fall back to HTTP only.
            let tls_certs = lighttpd_has_openssl(env).then_some(certs_str.as_str());
            crate::web::lighttpd::regenerate(
                &env.paths.etc(),
                &dashboard_dir,
                &env.paths.pids().join("lighttpd.pid").display().to_string(),
                &env.paths.logs().join("lighttpd.log").display().to_string(),
                &env.paths
                    .logs()
                    .join("lighttpd-access.log")
                    .display()
                    .to_string(),
                &sites,
                fpm_port,
                http_port,
                https_port,
                tls_certs,
            )?;
        }
        APACHE => {
            let bin = resolve_web_bin(env, APACHE)?;
            crate::web::apache::regenerate(
                &env.paths.etc(),
                &apache_server_root(&bin).display().to_string(),
                &env.paths.pids().join("httpd.pid").display().to_string(),
                &env.paths.logs().join("httpd.log").display().to_string(),
                &sites,
                fpm_port,
                http_port,
                https_port,
                Some(&certs_str),
            )?;
        }
        NGINX => {
            crate::web::nginx::regenerate(
                &env.paths.etc(),
                &env.paths.pids().join("nginx.pid").display().to_string(),
                &env.paths.logs().join("nginx.log").display().to_string(),
                &sites,
                fpm_port,
                http_port,
                https_port,
                Some(&certs_str),
            )?;
        }
        _ => {
            let tls_line = |s: &crate::site::Site| {
                let cert = certs_dir.join(format!("{}.crt", s.name));
                if cert.exists() {
                    format!(
                        "\ttls {} {}\n",
                        cert.display(),
                        certs_dir.join(format!("{}.key", s.name)).display()
                    )
                } else {
                    "\ttls internal\n".to_string()
                }
            };
            let mut caddyfile =
                generate_caddyfile_for_sites(&sites, fpm_port, http_port, https_port, &tls_line)?;
            caddyfile.push_str(&crate::web::caddyfile::dashboard_block(
                &dashboard_dir,
                http_port,
            ));
            std::fs::write(env.caddyfile_path(), caddyfile)?;
        }
    }
    let _ = certs_pair;
    let php_etc = env.php_etc_dir(php_version);
    std::fs::create_dir_all(&php_etc)?;
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
    let ini_path = env.php_ini_path(php_version);
    let socket = env.paths.pids().join(crate::orchestrator::MARIADB_SOCKET);
    // Always regenerate: this file is fidim-owned and must track the current
    // socket (e.g. after moving the data root), so `mysqli_connect('localhost')`
    // finds the managed MariaDB instead of a system socket.
    let _ = std::fs::write(
        &ini_path,
        crate::php::fpm::generate_php_ini(&[], Some(&socket)),
    );
    Ok(())
}

/// Start caddy + php-fpm for `php_version` (if not running) in detached mode.
pub fn start_all<R: Runner + Clone>(
    env: &Env,
    ports: &PortPlan,
    runner: &R,
    php_version: &str,
) -> Result<Vec<crate::service::ServiceStatus>> {
    start_all_with(
        env,
        ports,
        runner,
        php_version,
        &crate::proc::listening_pids,
    )
}

/// [`start_all`] with an injected port scan (tests must not touch sockets).
fn start_all_with<R: Runner + Clone>(
    env: &Env,
    ports: &PortPlan,
    runner: &R,
    php_version: &str,
    scan: crate::supervisor::PortScan<'_>,
) -> Result<Vec<crate::service::ServiceStatus>> {
    let specs = service_cmds(env, ports, php_version)?;
    let sup = Supervisor {
        env: env.clone(),
        ports: ports.clone(),
        runner: runner.clone(),
    };
    regenerate_ghosts(env, ports, php_version, runner)?;
    let fpm_port = fpm_port_for(php_version, ports.fpm_base, ports);
    let mut out = Vec::new();
    for (name, bin, args, log) in specs {
        if sup.status(&name).state == crate::service::ServiceState::Running {
            continue;
        }
        // orphaned php-fpm pool children survive master death without a pid
        // file: reclaim the port before spawning a new master
        sup.reap_orphans(&name, scan)?;
        check_port(fpm_port, &bin)?;
        let mut full = vec![bin];
        full.extend(args);
        let wrapped = with_log_append(&full, &log)?;
        let pid = sup.runner.spawn(&wrapped)?;
        sup.write_pid(&name, pid)?;
        ensure_started(&sup, &name, pid, &log, service_port(&name, ports))?;
        out.push(sup.status(&name));
    }
    Ok(out)
}

/// Start a single service by name (php-8.4, caddy, httpd, nginx, mariadb).
pub fn start_service<R: Runner + Clone>(
    env: &Env,
    ports: &PortPlan,
    runner: &R,
    php_version: &str,
    name: &str,
) -> Result<crate::service::ServiceStatus> {
    start_service_with(
        env,
        ports,
        runner,
        php_version,
        name,
        &crate::proc::listening_pids,
    )
}

/// [`start_service`] with an injected port scan (tests must not touch sockets).
fn start_service_with<R: Runner + Clone>(
    env: &Env,
    ports: &PortPlan,
    runner: &R,
    php_version: &str,
    name: &str,
    scan: crate::supervisor::PortScan<'_>,
) -> Result<crate::service::ServiceStatus> {
    if name == MARIADB {
        let sup = Supervisor {
            env: env.clone(),
            ports: ports.clone(),
            runner: runner.clone(),
        };
        if sup.status(MARIADB).state == crate::service::ServiceState::Running {
            return Ok(sup.status(MARIADB));
        }
        sup.reap_orphans(MARIADB, scan)?;
        let cnf = env.paths.etc().join("my.cnf");
        let datadir = env.paths.data().join("mariadb");
        let cmd = mariadb_config(env, &cnf, &datadir, &env.paths.pids(), ports.mariadb)?
            .ok_or_else(|| Error::Other("MariaDB binary was not resolved".into()))?;
        let log = env.paths.logs().join("mariadb.log");
        let wrapped = with_log_append(&cmd, &log)?;
        let pid = sup.runner.spawn(&wrapped)?;
        sup.write_pid(MARIADB, pid)?;
        ensure_started(&sup, MARIADB, pid, &log, Some(ports.mariadb))?;
        return Ok(sup.status(MARIADB));
    }
    let sup = Supervisor {
        env: env.clone(),
        ports: ports.clone(),
        runner: runner.clone(),
    };
    let fpm_port = fpm_port_for(php_version, ports.fpm_base, ports);
    let spec = if [CADDY, APACHE, NGINX, LIGHTTPD].contains(&name) {
        // Starting a named web server must not depend on the selected default
        // server being installed. Generate and resolve only this server.
        generate_web_configs(env, ports, php_version, name)?;
        resolve_web_bin(env, name).ok().map(|bin| {
            (
                name.to_string(),
                bin.display().to_string(),
                web_args(env, name),
                env.paths.logs().join(format!("{name}.log")),
            )
        })
    } else {
        generate_configs(env, ports, php_version)?;
        service_cmds(env, ports, php_version)?
            .into_iter()
            .find(|(n, ..)| n == name)
    };
    let (name, bin, args, log) =
        spec.ok_or_else(|| Error::Other(format!("unknown service {name}")))?;
    if [CADDY, APACHE, NGINX, LIGHTTPD].contains(&name.as_str())
        && php_binary_for(env, php_version).is_some()
    {
        ensure_php_fpm_started(env, ports, runner, php_version, scan)?;
    }
    sup.reap_orphans(&name, scan)?;
    check_port(fpm_port, &bin)?;
    let mut full = vec![bin];
    full.extend(args);
    let wrapped = with_log_append(&full, &log)?;
    let pid = sup.runner.spawn(&wrapped)?;
    sup.write_pid(&name, pid)?;
    ensure_started(&sup, &name, pid, &log, service_port(&name, ports))?;

    Ok(sup.status(&name))
}

fn ensure_php_fpm_started<R: Runner + Clone>(
    env: &Env,
    ports: &PortPlan,
    runner: &R,
    php_version: &str,
    scan: crate::supervisor::PortScan<'_>,
) -> Result<()> {
    let service = format!("php-{php_version}");
    let sup = Supervisor {
        env: env.clone(),
        ports: ports.clone(),
        runner: runner.clone(),
    };
    if sup.status(&service).state == crate::service::ServiceState::Running {
        return Ok(());
    }

    sup.reap_orphans(&service, scan)?;
    let bin = php_binary_for(env, php_version).ok_or_else(|| {
        Error::Other(format!(
            "php {php_version} not installed (fidim install php-fpm-{php_version})"
        ))
    })?;
    let fpm_port = fpm_port_for(php_version, ports.fpm_base, ports);
    check_port(fpm_port, &bin.display().to_string())?;
    let mut args = vec![
        "-F".to_string(),
        "-y".to_string(),
        env.fpm_conf_path(php_version).display().to_string(),
    ];
    let ini_path = env.php_ini_path(php_version);
    if ini_path.exists() {
        args.push("-c".to_string());
        args.push(ini_path.display().to_string());
    }
    let mut command = vec![bin.display().to_string()];
    command.extend(args);
    let log = env.paths.logs().join(format!("{service}.log"));
    let wrapped = with_log_append(&command, &log)?;
    let pid = sup.runner.spawn(&wrapped)?;
    sup.write_pid(&service, pid)?;
    ensure_started(&sup, &service, pid, &log, Some(fpm_port))?;
    Ok(())
}

/// Replay configs + reap ghost processes that own them (pid files were lost).
/// Services recorded in a live pid file are never touched: `fidim start` on an
/// already running stack must not restart it.
fn regenerate_ghosts<R: Runner>(
    env: &Env,
    ports: &PortPlan,
    php_version: &str,
    runner: &R,
) -> Result<()> {
    generate_configs(env, ports, php_version)?;
    let web = web_server(env);
    let conf_needle = web_config_path(env, &web).display().to_string();
    let fpm_needle = env.fpm_conf_path(php_version).display().to_string();
    let keep = live_service_pids(env, runner);
    let _ = reap_owners(&[conf_needle, fpm_needle], &keep);
    Ok(())
}

/// Pids recorded in a live pid file: services we started that must survive a
/// config replay.
fn live_service_pids<R: Runner>(env: &Env, runner: &R) -> Vec<u32> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(env.paths.pids()) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(base) = name.strip_suffix(".pid") else {
            continue;
        };
        if base.ends_with(".sock") {
            continue;
        }
        if let Ok(Some(pid)) = crate::proc::read_pid_file(&env.paths.pids(), base) {
            if runner.is_running(pid) {
                out.push(pid);
            }
        }
    }
    out
}

pub fn server_bin_path(env: &Env, server: &str) -> std::path::PathBuf {
    let nested = env.paths.bin().join(server).join(server);
    if nested.exists() {
        return nested;
    }
    let flat = env.paths.bin().join(server);
    if flat.is_file() {
        return flat;
    }
    if env.paths.bin().join(server).is_dir() {
        return nested;
    }
    flat
}

/// Binary path and CLI args per web server choice.
pub fn web_bin_path(env: &Env, web: &str) -> std::path::PathBuf {
    match web {
        NGINX => server_bin_path(env, "nginx"),
        APACHE => server_bin_path(env, "httpd"),
        LIGHTTPD => server_bin_path(env, "lighttpd"),
        _ => env.caddy_bin(),
    }
}

/// Every installed php version (managed installs + drop-in folders).
pub fn installed_php_versions(env: &Env) -> Vec<String> {
    let lock = crate::manifest::Lock::load(&env.lock_path()).unwrap_or_default();
    let mut out = crate::manifest::PhpManager.installed(&lock, &env.paths.bin());
    for v in crate::manifest::PhpManager.dropped_in(&env.paths.root().join("php")) {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `"<name> v<version>"`-style version probe of an installed web server.
pub fn web_version(env: &Env, web: &str) -> Option<String> {
    let bin = web_bin_path(env, web);
    let bin = if bin.exists() {
        bin
    } else {
        let flat = env.paths.bin().join(web);
        if flat.exists() {
            flat
        } else {
            bin
        }
    };
    let out = match web {
        APACHE | NGINX | LIGHTTPD => std::process::Command::new(&bin).arg("-v").output().ok()?,
        _ => std::process::Command::new(&bin)
            .arg("--version")
            .output()
            .ok()?,
    };
    let txt =
        String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    parse_web_version(&txt)
}

fn parse_web_version(output: &str) -> Option<String> {
    output
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
        .find_map(|token| {
            let version = token
                .strip_prefix('v')
                .or_else(|| token.strip_prefix('V'))
                .unwrap_or(token);
            let parts: Vec<_> = version.split('.').collect();
            if parts.len() < 2
                || parts
                    .iter()
                    .any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()))
            {
                return None;
            }
            Some(parts.into_iter().take(3).collect::<Vec<_>>().join("."))
        })
}

/// Data-root binary when present, otherwise `PATH` fallback; error when
/// neither exists.
pub fn resolve_web_bin(env: &Env, web: &str) -> Result<std::path::PathBuf> {
    if ![CADDY, NGINX, APACHE, LIGHTTPD].contains(&web) {
        return Err(Error::Other(format!("unknown web server: {web}")));
    }
    let local = web_bin_path(env, web);
    if local.exists() {
        return Ok(local);
    }
    let flat = env.paths.bin().join(web);
    if flat.exists() {
        return Ok(flat);
    }
    if let Some(sys) = which_system(web) {
        return Ok(sys);
    }
    Err(Error::Other(format!(
        "missing {web} binary: install via `fidim install {web}` or ensure it is on PATH"
    )))
}

/// Absolute `PATH`-resolved binary name (httpd/nginx/caddy); None when absent
/// from PATH or data/bin.
/// php-fpm binary for `version`: `bin/php/php-fpm-<v>` or `bin/php-fpm-<v>` first,
/// then a drop-in folder under `<root>/php/<v>/` (php-fpm / bin/php-fpm / sbin/php-fpm).
pub fn php_binary_for(env: &Env, version: &str) -> Option<std::path::PathBuf> {
    for dir in [env.paths.bin().join("php"), env.paths.bin()] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if let Some(v) = n.strip_prefix("php-fpm-") {
                    if crate::manifest::short_version(v) == version {
                        return Some(e.path());
                    }
                }
            }
        }
    }
    crate::manifest::php_binary_in(&env.paths.root().join("php").join(version))
}

pub fn which_system(bin: &str) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("/usr/bin/which")
        .arg(bin)
        .output()
        .ok()?;
    if out.status.success() {
        let p = String::from_utf8_lossy(&out.stdout);
        let p = p.trim();
        (!p.is_empty()).then(|| PathBuf::from(p))
    } else {
        None
    }
}

pub fn web_config_path(env: &Env, web: &str) -> std::path::PathBuf {
    match web {
        NGINX => env.paths.etc().join("nginx/nginx.conf"),
        APACHE => env.paths.etc().join("apache/httpd.conf"),
        LIGHTTPD => env.paths.etc().join("lighttpd/lighttpd.conf"),
        _ => env.caddyfile_path(),
    }
}

/// Directory that contains `modules/` next to an httpd binary.
/// Searches up to 3 levels above the binary so it works with multiple layouts:
/// - XAMPP: `<prefix>/bin/httpd` → `<prefix>/modules/`  (2 levels up)
/// - fidim data root: `bin/httpd/httpd` → `modules/`  (3 levels up)
/// - Simple flat install: `<prefix>/httpd` → `<prefix>/modules/`  (1 level up)
pub fn apache_server_root(bin: &std::path::Path) -> std::path::PathBuf {
    let mut cur = bin.parent().unwrap_or(Path::new("."));
    for _ in 0..3 {
        if cur.join("modules").is_dir() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }
    bin.parent().unwrap_or(Path::new(".")).to_path_buf()
}

pub fn web_args(env: &Env, web: &str) -> Vec<String> {
    let conf = web_config_path(env, web).display().to_string();
    match web {
        // The downloaded nginx/lighttpd binaries are built with the old data
        // root compiled in, so point them at this root's prefix/module dir.
        NGINX => vec![
            "-c".into(),
            conf,
            "-p".into(),
            env.paths.root().join("nginx").display().to_string(),
        ],
        APACHE => vec!["-f".into(), conf, "-DFOREGROUND".into()],
        LIGHTTPD => vec![
            "-D".into(),
            "-f".into(),
            conf,
            "-m".into(),
            lighttpd_module_dir(env).display().to_string(),
        ],
        _ => vec!["run".into(), "--config".into(), conf],
    }
}

/// Resolve Lighttpd's dynamic module directory across source-build layouts.
/// CMake installs modules below `lib/lighttpd`, while older/manual installs
/// may place them directly below `lib`.
pub fn lighttpd_module_dir(env: &Env) -> PathBuf {
    let nested = env.paths.root().join("lighttpd/lib/lighttpd");
    if nested.is_dir() {
        nested
    } else {
        env.paths.root().join("lighttpd/lib")
    }
}

fn lighttpd_has_openssl(env: &Env) -> bool {
    lighttpd_module_dir(env).join("mod_openssl.so").is_file()
}

/// How long a freshly spawned service must stay alive before it is treated as
/// started. A bind failure exits within a few milliseconds.
const STARTUP_POLL: std::time::Duration = std::time::Duration::from_millis(20);
const STARTUP_ALIVE_POLLS: usize = 10;
/// Upper bound for the port actually appearing (servers fork workers first).
const STARTUP_PORT_POLLS: usize = 60;
/// MariaDB runs InnoDB recovery before it binds, which can take several
/// seconds right after a restart.
const STARTUP_PORT_POLLS_DB: usize = 750;

/// A service that exits (or never binds its port) right after spawn would
/// otherwise show as "Running" while serving nothing. Report it, clean up the
/// pid file and point at the port setting.
fn ensure_started<R: Runner>(
    sup: &Supervisor<R>,
    name: &str,
    pid: u32,
    log: &Path,
    port: Option<u16>,
) -> Result<()> {
    let mut polls = 0;
    loop {
        std::thread::sleep(STARTUP_POLL);
        polls += 1;
        if !sup.runner.is_running(pid) {
            crate::proc::remove_pid_file(&sup.env.paths.pids(), name);
            return Err(start_error(
                name,
                log,
                "exited right after start; its port may already be in use",
            ));
        }
        match port {
            Some(p) if sup.runner.owns_port(pid, p) => return Ok(()),
            Some(_) => {}
            None if polls >= STARTUP_ALIVE_POLLS => return Ok(()),
            None => {}
        }
        let limit = if name == MARIADB {
            STARTUP_PORT_POLLS_DB
        } else {
            STARTUP_PORT_POLLS
        };
        if polls >= limit {
            let _ = sup.runner.kill(pid);
            crate::proc::remove_pid_file(&sup.env.paths.pids(), name);
            return Err(start_error(
                name,
                log,
                &format!(
                    "is running but never listened on port {}; the port may already be in use",
                    port.unwrap_or_default()
                ),
            ));
        }
    }
}

fn start_error(name: &str, log: &Path, reason: &str) -> Error {
    let detail = last_log_line(log)
        .map(|line| format!("\nlast line of {}: {line}", log.display()))
        .unwrap_or_default();
    Error::Other(format!(
        "{name} {reason}. Choose another port with `fidim ports set <key> <port>`.{detail}"
    ))
}

/// The TCP port a service must own once started.
fn service_port(name: &str, ports: &PortPlan) -> Option<u16> {
    if name == MARIADB {
        Some(ports.mariadb)
    } else if let Some(version) = name.strip_prefix("php-") {
        Some(fpm_port_for(version, ports.fpm_base, ports))
    } else if [CADDY, APACHE, NGINX, LIGHTTPD].contains(&name) {
        Some(ports.web_ports(name).0)
    } else {
        None
    }
}

fn last_log_line(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
}

/// Fail early with a helpful message and alternative port when something
/// already occupies the HTTP port (system Apache/nginx, another stack, …).
fn check_port(_port: u16, bin: &str) -> Result<()> {
    if !Path::new(bin).exists() {
        return Err(Error::Other(format!("missing binary {bin}")));
    }
    Ok(())
}

/// Stop a single named service (php-<ver>, mariadb, or a web server).
pub fn stop_service<R: Runner + Clone>(
    env: &Env,
    ports: &PortPlan,
    runner: &R,
    name: &str,
) -> Result<()> {
    Supervisor {
        env: env.clone(),
        ports: ports.clone(),
        runner: runner.clone(),
    }
    .stop(name)
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
    // Keep every supported web server visible, including those not installed,
    // so the TUI can expose their install/use actions.
    for w in [CADDY, APACHE, NGINX, LIGHTTPD] {
        if !names.iter().any(|n| n == w) {
            names.push(w.to_string());
        }
    }
    for v in installed_php_versions(env) {
        let n = format!("php-{v}");
        if !names.iter().any(|e| e == &n) {
            names.push(n);
        }
    }
    let lock = Lock::load(&env.lock_path()).unwrap_or_default();
    let web = web_server(env);
    let php = format!("php-{}", default_php(&lock));
    for n in [web.as_str(), php.as_str(), MARIADB] {
        if !names.iter().any(|e| e == n) {
            names.push(n.to_string());
        }
    }
    for &l in crate::lang::SUPPORTED_LANGUAGES {
        if l == "php" {
            continue;
        }
        let installed = crate::lang::installed_lang_versions(env, l);
        if !installed.is_empty() {
            for v in installed {
                let n = format!("{l}-{v}");
                if !names.iter().any(|e| e == &n) {
                    names.push(n);
                }
            }
        }
        if !names
            .iter()
            .any(|e| e == l || e.starts_with(&format!("{l}-")))
        {
            names.push(l.to_string());
        }
    }
    names.iter().map(|n| sup.status(n)).collect()
}

/// List folders under the managed projects directory (the TUI project list).
pub fn list_projects(env: &Env) -> Vec<String> {
    list_sites(env).into_iter().map(|s| s.name).collect()
}

/// Projects under `Sites` paired with the folder the web server should serve.
pub fn list_sites(env: &Env) -> Vec<crate::site::Site> {
    let root = env.paths.sites();
    if let Ok(rd) = std::fs::read_dir(root) {
        let mut v: Vec<crate::site::Site> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                crate::site::Site::detect(&name, root)
            })
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        return v;
    }
    Vec::new()
}

#[cfg(test)]
mod webchoice_tests {
    use super::*;

    #[test]
    fn default_web_falls_back_to_caddy() {
        assert_eq!(default_web(&Lock::default()), CADDY);
        let lock = Lock {
            web_default: Some(NGINX.into()),
            ..Lock::default()
        };
        assert_eq!(default_web(&lock), NGINX);
    }

    #[test]
    fn default_web_ignores_invalid_persisted_server_name() {
        let lock = Lock {
            web_default: Some("caddy  [2.10.210839268]".into()),
            ..Lock::default()
        };

        assert_eq!(default_web(&lock), CADDY);
    }

    #[test]
    fn resolve_web_bin_rejects_unknown_server_names() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().to_path_buf(), t.path().join("Sites"));
        std::fs::create_dir_all(env.paths.bin().join(CADDY)).unwrap();
        std::fs::write(env.caddy_bin(), "fake caddy").unwrap();

        assert!(resolve_web_bin(&env, "caddy  [2.10.210839268]").is_err());
    }

    #[test]
    fn web_paths_and_args_per_server() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().to_path_buf(), t.path().join("Sites"));
        assert!(web_config_path(&env, CADDY)
            .display()
            .to_string()
            .contains("Caddyfile"));
        let apache = web_config_path(&env, APACHE);
        assert!(apache.display().to_string().ends_with("apache/httpd.conf"));
        let nginx = web_config_path(&env, NGINX);
        assert!(nginx.display().to_string().ends_with("nginx/nginx.conf"));
        assert_eq!(
            web_args(&env, APACHE),
            vec![
                "-f".to_string(),
                apache.display().to_string(),
                "-DFOREGROUND".to_string()
            ]
        );
        assert_eq!(
            web_args(&env, NGINX),
            vec![
                "-c".to_string(),
                nginx.display().to_string(),
                "-p".to_string(),
                env.paths.root().join("nginx").display().to_string(),
            ]
        );
        let lighttpd = web_config_path(&env, LIGHTTPD);
        assert_eq!(
            web_args(&env, LIGHTTPD),
            vec![
                "-D".to_string(),
                "-f".to_string(),
                lighttpd.display().to_string(),
                "-m".to_string(),
                env.paths.root().join("lighttpd/lib").display().to_string(),
            ]
        );
        assert_eq!(
            web_args(&env, CADDY),
            vec![
                "run".to_string(),
                "--config".to_string(),
                env.caddyfile_path().display().to_string()
            ]
        );
    }

    #[test]
    fn apache_server_root_prefers_modules_dir() {
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("opt/httpd/bin/httpd");
        std::fs::create_dir_all(t.path().join("opt/httpd/bin")).unwrap();
        std::fs::create_dir_all(t.path().join("opt/httpd/modules")).unwrap();
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        assert_eq!(apache_server_root(&bin), t.path().join("opt/httpd"));
    }

    #[test]
    fn apache_server_root_finds_modules_three_levels_up() {
        // Nested installs are supported when modules live three levels above the binary.
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join(".fidim/bin/httpd/httpd");
        std::fs::create_dir_all(t.path().join(".fidim/bin/httpd")).unwrap();
        std::fs::create_dir_all(t.path().join(".fidim/modules")).unwrap();
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        assert_eq!(apache_server_root(&bin), t.path().join(".fidim"));
    }
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

    /// Spawn "succeeds" but the process is gone right away, as when a server
    /// cannot bind its port.
    #[derive(Clone, Default)]
    struct DyingRunner;

    impl Runner for DyingRunner {
        fn spawn(&self, _cmd: &[String]) -> std::result::Result<u32, std::io::Error> {
            Ok(4242)
        }
        fn is_running(&self, _pid: u32) -> bool {
            false
        }
        fn kill(&self, _pid: u32) -> std::result::Result<(), std::io::Error> {
            Ok(())
        }
    }

    /// Spawn stays alive (like an nginx master retrying a failed bind) but the
    /// port is owned by another process.
    #[derive(Clone, Default)]
    struct PortlessRunner;

    impl Runner for PortlessRunner {
        fn spawn(&self, _cmd: &[String]) -> std::result::Result<u32, std::io::Error> {
            Ok(4243)
        }
        fn is_running(&self, _pid: u32) -> bool {
            true
        }
        fn kill(&self, _pid: u32) -> std::result::Result<(), std::io::Error> {
            Ok(())
        }
        fn owns_port(&self, _pid: u32, _port: u16) -> bool {
            false
        }
    }

    #[derive(Clone, Default)]
    struct PhpFailingRunner {
        commands: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Runner for PhpFailingRunner {
        fn spawn(&self, cmd: &[String]) -> std::result::Result<u32, std::io::Error> {
            let command = cmd.join(" ");
            self.commands.lock().unwrap().push(command.clone());
            if command.contains("php-fpm-8.4") {
                return Err(std::io::Error::other("PHP-FPM could not start"));
            }
            Ok(5000)
        }

        fn is_running(&self, _pid: u32) -> bool {
            true
        }

        fn kill(&self, _pid: u32) -> std::result::Result<(), std::io::Error> {
            Ok(())
        }
    }

    #[test]
    fn start_reports_a_service_that_never_binds_and_clears_its_pid() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        std::fs::write(env.caddy_bin(), "#!/bin/sh\n").unwrap();
        std::fs::write(env.paths.bin().join("php-fpm-8.4"), "#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();

        let err = start_all_with(&env, &PortPlan::default(), &PortlessRunner, "8.4", &|_| {
            Ok(vec![])
        })
        .unwrap_err();

        let msg = format!("{err}");
        assert!(msg.contains("listened"), "{msg}");
        assert!(msg.contains("port"), "{msg}");
        assert!(
            !env.paths.pids().join("caddy.pid").exists(),
            "a service that never bound must not keep a pid file"
        );
    }

    #[test]
    fn start_reports_a_service_that_exits_immediately_and_removes_its_pid() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        std::fs::write(env.caddy_bin(), "#!/bin/sh\n").unwrap();
        std::fs::write(env.paths.bin().join("php-fpm-8.4"), "#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();

        let err = start_all_with(&env, &PortPlan::default(), &DyingRunner, "8.4", &|_| {
            Ok(vec![])
        })
        .unwrap_err();

        let msg = format!("{err}");
        assert!(
            msg.contains("caddy"),
            "must name the failing service: {msg}"
        );
        assert!(
            msg.contains("port"),
            "must point at the occupied port: {msg}"
        );
        assert!(
            !env.paths.pids().join("caddy.pid").exists(),
            "a dead service must not keep a pid file"
        );
    }

    #[test]
    fn web_version_parses_caddy_semver_without_build_hash() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        std::fs::create_dir_all(env.paths.bin().join("caddy")).unwrap();
        let bin = env.caddy_bin();
        std::fs::write(&bin, "#!/bin/sh\necho 'v2.10.2 h1:210839268'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let got = crate::orchestrator::web_version(&env, CADDY);

        assert_eq!(got.as_deref(), Some("2.10.2"));
    }

    #[test]
    fn web_version_parses_apache_version_from_dash_v() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        let bin = env.paths.bin().join("httpd/httpd");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "#!/bin/sh\nif [ \"$1\" = \"-v\" ]; then echo 'Server version: Apache/2.4.67'; else exit 1; fi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let got = crate::orchestrator::web_version(&env, APACHE);

        assert_eq!(got.as_deref(), Some("2.4.67"));
    }

    #[test]
    fn generate_configs_binds_nginx_to_its_own_ports() {
        let (env, _t) = env();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        let lock = Lock {
            web_default: Some(NGINX.into()),
            ..Lock::default()
        };
        lock.save(&env.lock_path()).unwrap();
        let ports = PortPlan {
            nginx_http: 8280,
            nginx_https: 8445,
            ..PortPlan::default()
        };
        generate_configs(&env, &ports, "8.4").unwrap();
        let site = std::fs::read_to_string(env.paths.etc().join("nginx/sites/blog.conf")).unwrap();
        assert!(
            site.contains("listen 8280;"),
            "nginx must bind its own http port, not the shared one\n{site}"
        );
        assert!(
            site.contains("listen 8445 "),
            "nginx https port missing\n{site}"
        );
    }

    #[test]
    fn generate_configs_binds_caddy_to_its_own_ports() {
        let (env, _t) = env();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        let ports = PortPlan {
            caddy_http: 8180,
            caddy_https: 8480,
            ..PortPlan::default()
        };
        generate_configs(&env, &ports, "8.4").unwrap();
        let cf = std::fs::read_to_string(env.caddyfile_path()).unwrap();
        assert!(cf.contains("blog.test:8180"), "http block missing\n{cf}");
        assert!(cf.contains("blog.test:8480"), "https block missing\n{cf}");
    }

    #[test]
    fn caddy_serves_dashboard_when_no_projects_exist() {
        let (env, _t) = env();
        let ports = PortPlan::default();

        generate_configs(&env, &ports, "8.4").unwrap();

        let cf = std::fs::read_to_string(env.caddyfile_path()).unwrap();
        let dashboard = env.paths.etc().join("dashboard");
        assert!(
            cf.contains(&format!("http://:{}", ports.caddy_http)),
            "caddy needs a catch-all listener for the dashboard\n{cf}"
        );
        assert!(
            cf.contains(&format!("root * {}", dashboard.display())),
            "dashboard document root missing\n{cf}"
        );
        assert!(
            dashboard.join("index.html").exists(),
            "shared dashboard asset missing"
        );
    }

    #[test]
    fn nginx_serves_dashboard_when_no_projects_exist() {
        let (env, _t) = env();
        let lock = Lock {
            web_default: Some(NGINX.into()),
            ..Lock::default()
        };
        lock.save(&env.lock_path()).unwrap();
        let ports = PortPlan::default();

        generate_configs(&env, &ports, "8.4").unwrap();

        let conf = std::fs::read_to_string(env.paths.etc().join("nginx/nginx.conf")).unwrap();
        let dashboard = env.paths.etc().join("dashboard");
        assert!(
            conf.contains(&format!("listen {} default_server;", ports.nginx_http)),
            "nginx needs a default server for the dashboard\n{conf}"
        );
        assert!(
            conf.contains(&format!("root {};", dashboard.display())),
            "dashboard document root missing\n{conf}"
        );
        assert!(
            dashboard.join("index.html").exists(),
            "shared dashboard asset missing"
        );
    }

    #[test]
    fn lighttpd_serves_dashboard_when_no_projects_exist() {
        let (env, _t) = env();
        let lock = Lock {
            web_default: Some(LIGHTTPD.into()),
            ..Lock::default()
        };
        lock.save(&env.lock_path()).unwrap();
        let ports = PortPlan::default();

        generate_configs(&env, &ports, "8.4").unwrap();

        let conf = std::fs::read_to_string(env.paths.etc().join("lighttpd/lighttpd.conf")).unwrap();
        let dashboard = env.paths.etc().join("dashboard");
        assert!(
            conf.contains(&format!(
                "server.document-root = \"{}\"",
                dashboard.display()
            )),
            "lighttpd must fall back to the dashboard\n{conf}"
        );
        assert!(
            conf.contains("index-file.names = ( \"index.html\" )"),
            "dashboard index must be served\n{conf}"
        );
        assert!(
            dashboard.join("index.html").exists(),
            "shared dashboard asset missing"
        );
    }

    #[test]
    fn lighttpd_skips_tls_when_openssl_module_is_missing() {
        let (env, _t) = env();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        let lock = Lock {
            web_default: Some(LIGHTTPD.into()),
            ..Lock::default()
        };
        lock.save(&env.lock_path()).unwrap();

        generate_configs(&env, &PortPlan::default(), "8.4").unwrap();

        let conf = std::fs::read_to_string(env.paths.etc().join("lighttpd/lighttpd.conf")).unwrap();
        assert!(
            !conf.contains("mod_openssl"),
            "openssl must not load when the module is unavailable\n{conf}"
        );
        assert!(
            !conf.contains("_ssl.conf"),
            "TLS socket include must be skipped without the module\n{conf}"
        );
    }

    #[test]
    fn lighttpd_module_dir_prefers_the_nested_lib_directory() {
        let (env, _t) = env();
        let nested = env.paths.root().join("lighttpd/lib/lighttpd");
        std::fs::create_dir_all(&nested).unwrap();

        let args = web_args(&env, LIGHTTPD);

        assert!(
            args.contains(&nested.display().to_string()),
            "lighttpd modules live in lib/lighttpd: {args:?}"
        );
    }

    #[test]
    fn lighttpd_module_dir_falls_back_to_the_flat_lib_directory() {
        let (env, _t) = env();
        let flat = env.paths.root().join("lighttpd/lib");
        std::fs::create_dir_all(&flat).unwrap();

        let args = web_args(&env, LIGHTTPD);

        assert!(
            args.contains(&flat.display().to_string()),
            "flat module directory must keep working: {args:?}"
        );
    }

    #[test]
    fn generate_configs_serves_framework_projects_from_public() {
        let (env, _t) = env();
        std::fs::create_dir_all(env.paths.sites().join("shop/public")).unwrap();
        std::fs::write(env.paths.sites().join("shop/public/index.php"), "<?php").unwrap();
        let lock = Lock {
            web_default: Some(NGINX.into()),
            ..Lock::default()
        };
        lock.save(&env.lock_path()).unwrap();

        generate_configs(&env, &PortPlan::default(), "8.4").unwrap();

        let site = std::fs::read_to_string(env.paths.etc().join("nginx/sites/shop.conf")).unwrap();
        assert!(
            site.contains(&format!(
                "root {};",
                env.paths.sites().join("shop/public").display()
            )),
            "nginx must serve public/, not the project root\n{site}"
        );
    }

    #[test]
    fn generate_configs_serves_phpmyadmin_when_installed() {
        let (env, _t) = env();
        let pma = env.paths.runtimes().join("phpmyadmin/5.2.3");
        std::fs::create_dir_all(&pma).unwrap();
        std::fs::write(pma.join("index.php"), "<?php").unwrap();

        generate_configs(&env, &PortPlan::default(), "8.4").unwrap();

        let cf = std::fs::read_to_string(env.caddyfile_path()).unwrap();
        assert!(cf.contains("phpmyadmin.test"), "vhost missing\n{cf}");
        assert!(
            env.paths.sites().join("phpmyadmin").exists(),
            "site link missing"
        );
        assert!(pma.join("config.inc.php").exists(), "config missing");
    }

    #[test]
    fn start_service_for_non_chosen_web_binds_its_own_ports() {
        let (env, _t) = env();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        // nginx installed locally while caddy stays the chosen default
        std::fs::create_dir_all(env.paths.bin()).unwrap();
        let bin = env.paths.bin().join("nginx");
        std::fs::write(&bin, "#!/bin/sh\nexec sleep 999\n").unwrap();
        let caddy = env.paths.bin().join("caddy");
        std::fs::write(&caddy, "#!/bin/sh\nexec sleep 999\n").unwrap();
        let fpm = env.paths.bin().join("php-fpm-8.4");
        std::fs::write(&fpm, "#!/bin/sh\nexec sleep 999\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for b in [&bin, &caddy, &fpm] {
                let _ = std::fs::set_permissions(b, std::fs::Permissions::from_mode(0o755));
            }
        }
        let ports = PortPlan {
            nginx_http: 8281,
            nginx_https: 8446,
            ..PortPlan::default()
        };
        let f = FakeRunner::default();
        start_service(&env, &ports, &f, "8.4", NGINX).unwrap();
        assert!(
            env.paths.pids().join("php-8.4.pid").exists(),
            "starting a web server must also start the selected PHP-FPM service"
        );
        let site = std::fs::read_to_string(env.paths.etc().join("nginx/sites/blog.conf")).unwrap();
        assert!(
            site.contains("listen 8281;"),
            "non-chosen web must bind its own port\n{site}"
        );
        assert!(site.contains("listen 8446 "), "{site}");
    }

    #[test]
    fn starting_named_web_does_not_require_missing_default_web() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        let nginx = env.paths.bin().join("nginx");
        std::fs::write(&nginx, "#!/bin/sh\n").unwrap();

        let runner = FakeRunner::default();
        let status = start_service(&env, &PortPlan::default(), &runner, "8.4", NGINX).unwrap();

        assert_eq!(status.name, NGINX);
        assert!(!env.paths.pids().join("caddy.pid").exists());
    }

    #[test]
    fn starting_web_server_reports_automatic_php_fpm_start_failure() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        std::fs::write(env.caddy_bin(), "#!/bin/sh\n").unwrap();
        std::fs::write(env.paths.bin().join("php-fpm-8.4"), "#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        let runner = PhpFailingRunner::default();

        let result = start_service(&env, &PortPlan::default(), &runner, "8.4", CADDY);

        assert!(
            result.is_err(),
            "starting the web server must report if its automatic PHP-FPM start fails"
        );
    }

    #[test]
    fn starting_web_server_without_php_starts_the_web_service() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        std::fs::write(env.caddy_bin(), "#!/bin/sh\n").unwrap();

        let runner = FakeRunner::default();
        let status = start_service(&env, &PortPlan::default(), &runner, "8.4", CADDY).unwrap();

        assert_eq!(status.name, CADDY);
        assert_eq!(status.state, crate::service::ServiceState::Running);
        assert!(
            !env.paths.pids().join("php-8.4.pid").exists(),
            "PHP-FPM must remain stopped when it is not installed"
        );
    }

    #[test]
    fn status_all_reports_stopped_when_no_pids() {
        let (env, _t) = env();
        let f = FakeRunner::default();
        let st = status_all(&env, &PortPlan::default(), &f);
        assert!(st
            .iter()
            .filter(|s| { [CADDY, APACHE, NGINX, LIGHTTPD].contains(&s.name.as_str()) })
            .all(|s| s.state == crate::service::ServiceState::Stopped));
    }

    #[test]
    fn starting_a_php_service_reaps_workers_left_by_a_dead_master() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        std::fs::write(env.caddy_bin(), "#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        std::fs::write(env.paths.bin().join("php-fpm-8.4"), "#!/bin/sh\n").unwrap();
        // the master is gone but its pool workers still hold the fpm port
        let f = FakeRunner::default();
        f.state.spawned.lock().unwrap().push(999);
        std::fs::write(env.paths.pids().join("php-8.4.pid"), "999").unwrap();

        start_service_with(&env, &PortPlan::default(), &f, "8.4", "php-8.4", &|_| {
            Ok(vec![999])
        })
        .unwrap();

        assert!(
            f.state.kill_info.lock().unwrap().contains(&999),
            "orphan workers must be reaped before spawning a new master"
        );
        let pid = std::fs::read_to_string(env.paths.pids().join("php-8.4.pid")).unwrap();
        assert_ne!(pid.trim(), "999", "the stale pid file must be replaced");
    }

    #[test]
    fn start_service_runs_managed_mariadb_without_php_or_web_binaries() {
        let (env, _t) = env();
        let basedir = env.paths.runtimes().join("mariadb/11.4.13");
        let server = basedir.join("bin/mariadbd");
        std::fs::create_dir_all(server.parent().unwrap()).unwrap();
        std::fs::write(&server, "managed mariadbd").unwrap();
        std::fs::create_dir_all(env.paths.data().join("mariadb/mysql")).unwrap();

        let runner = FakeRunner::default();
        let status = start_service(&env, &PortPlan::default(), &runner, "8.4", MARIADB).unwrap();
        assert_eq!(status.name, MARIADB);
        assert_eq!(status.state, crate::service::ServiceState::Running);
        let config = std::fs::read_to_string(env.paths.etc().join("my.cnf")).unwrap();
        assert!(config.contains(&format!("basedir = {}", basedir.display())));
        assert!(config.contains("socket = "));
    }

    #[test]
    fn status_all_shows_every_supported_web_server() {
        // Apache is absent in the temp env, but all choices remain listed.
        let (env, _t) = env();
        let lock = Lock {
            web_default: Some(APACHE.into()),
            ..Lock::default()
        };
        lock.save(&env.lock_path()).unwrap();
        let f = FakeRunner::default();
        let st = status_all(&env, &PortPlan::default(), &f);
        let names: Vec<_> = st.iter().map(|s| s.name.as_str().to_string()).collect();
        for web in [CADDY, APACHE, NGINX, LIGHTTPD] {
            assert!(
                names.iter().any(|name| name == web),
                "{web} should be listed"
            );
        }
    }

    #[test]
    fn status_all_shows_uninstalled_web_server_with_stale_pid_file() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        std::fs::write(env.paths.pids().join("nginx.pid"), "999").unwrap();

        let statuses = status_all(&env, &PortPlan::default(), &FakeRunner::default());
        let nginx = statuses.iter().find(|status| status.name == NGINX);

        assert!(
            nginx.is_some(),
            "an uninstalled web server must still appear in the list"
        );
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
        assert!(env.php_ini_path(php_version).exists());
        let ini = std::fs::read_to_string(env.php_ini_path(php_version)).unwrap();
        assert!(ini.contains("memory_limit = 512M"));
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
    fn live_service_pids_keeps_only_processes_that_are_still_running() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        let f = FakeRunner::default();
        let alive = f.spawn(&["x".to_string()]).unwrap();
        crate::proc::write_pid_file(&env.paths.pids(), "caddy", alive).unwrap();
        crate::proc::write_pid_file(&env.paths.pids(), "php-8.4", 999).unwrap();

        assert_eq!(live_service_pids(&env, &f), vec![alive]);
    }

    #[test]
    fn start_all_is_idempotent_and_never_restarts_running_services() {
        let (env, _t) = env();
        env.ensure_dirs().unwrap();
        std::fs::write(env.caddy_bin(), "#!/bin/sh\n").unwrap();
        std::fs::write(env.paths.bin().join("php-fpm-8.4"), "#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        let f = FakeRunner::default();

        start_all(&env, &PortPlan::default(), &f, "8.4").unwrap();
        let spawned_after_first = f.state.spawned.lock().unwrap().len();
        start_all(&env, &PortPlan::default(), &f, "8.4").unwrap();

        assert_eq!(
            f.state.spawned.lock().unwrap().len(),
            spawned_after_first,
            "a second start must not spawn the stack again"
        );
        assert!(
            f.state.kill_info.lock().unwrap().is_empty(),
            "a second start must not kill the running stack"
        );
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
    fn list_sites_points_framework_projects_at_their_public_folder() {
        let (env, _t) = env();
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        std::fs::write(env.paths.sites().join("blog/index.php"), "<?php").unwrap();
        std::fs::create_dir_all(env.paths.sites().join("shop/public")).unwrap();
        std::fs::write(env.paths.sites().join("shop/public/index.php"), "<?php").unwrap();

        let sites = list_sites(&env);

        assert_eq!(sites.len(), 2, "{sites:?}");
        assert_eq!(sites[0].name, "blog");
        assert_eq!(sites[0].docroot, env.paths.sites().join("blog"));
        assert_eq!(sites[1].name, "shop");
        assert_eq!(sites[1].docroot, env.paths.sites().join("shop/public"));
    }

    #[test]
    fn list_projects_handles_missing_sites_dir() {
        let (env, _t) = env();
        assert!(list_projects(&env).is_empty());
    }
}

/// Add MariaDB config + start command when a managed or system server is usable.
pub fn mariadb_config(
    env: &Env,
    cnf_path: &Path,
    datadir: &Path,
    socket_dir: &Path,
    port: u16,
) -> Result<Option<Vec<String>>> {
    env.ensure_dirs()?;
    let (bin, basedir) = resolve_mariadb(env).ok_or_else(|| {
        Error::Other("MariaDB not found; install with `fidim install mariadb`".into())
    })?;
    std::fs::create_dir_all(socket_dir)?;
    let socket = socket_dir.join("db.sock");
    let log = env.paths.logs().join("mariadb.log");
    let cnf = generate_mariadb_cnf(
        basedir.display().to_string().trim_end_matches('/'),
        datadir.display().to_string().trim_end_matches('/'),
        socket.display().to_string().trim_end_matches('/'),
        port,
        log.display().to_string().trim_end_matches('/'),
        &std::env::var("USER").unwrap_or_default(),
        "127.0.0.1",
    )?;
    std::fs::write(cnf_path, &cnf)?;
    if datadir_missing(datadir) {
        init_datadir(env, &basedir, datadir)?;
    }
    Ok(Some(mariadb_start_cmd(&bin, &basedir, cnf_path)))
}

/// Order MariaDB runtime directory names (`11.4.13`, `9.11.2`, ...) by numeric
/// version components instead of lexically, so `11.4` beats `9.11`.
fn version_key(name: &std::ffi::OsStr) -> Vec<u64> {
    name.to_string_lossy()
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

/// Resolve a FIDIM-managed MariaDB first, then retain support for system installs.
fn resolve_mariadb(env: &Env) -> Option<(PathBuf, PathBuf)> {
    let runtime_root = env.paths.runtimes().join("mariadb");
    if let Ok(versions) = std::fs::read_dir(&runtime_root) {
        let mut versions: Vec<_> = versions
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .collect();
        versions.sort_by_key(|entry| version_key(&entry.file_name()));
        for version in versions.into_iter().rev() {
            let basedir = version.path();
            let managed_bin = basedir.join("bin/mariadbd");
            if managed_bin.is_file() {
                return Some((managed_bin, basedir));
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in ["mariadbd", "mysqld"] {
            let bin = dir.join(name);
            if bin.is_file() {
                let base = bin
                    .parent()
                    .and_then(Path::parent)
                    .unwrap_or_else(|| Path::new("/usr"))
                    .to_path_buf();
                return Some((bin, base));
            }
        }
    }
    None
}

/// Locate MariaDB in the selected data root or on PATH.
pub fn find_mariadbd(env: &Env) -> Option<PathBuf> {
    resolve_mariadb(env).map(|(bin, _)| bin)
}
fn init_datadir(env: &Env, basedir: &Path, datadir: &Path) -> Result<()> {
    let server_bin = resolve_mariadb(env)
        .map(|(bin, _)| bin)
        .ok_or_else(|| Error::Other("MariaDB server binary is missing".into()))?;
    let mut candidates = vec![
        basedir.join("scripts/mariadb-install-db"),
        basedir.join("bin/mariadb-install-db"),
        basedir.join("bin/mysql_install_db"),
        server_bin.with_file_name("mariadb-install-db"),
        server_bin.with_file_name("mysql_install_db"),
    ];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join("mariadb-install-db"));
            candidates.push(dir.join("mysql_install_db"));
        }
    }
    let init = candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            Error::Other(format!(
                "MariaDB initializer not found under {} or on PATH",
                basedir.display()
            ))
        })?;
    let lib = basedir.join("lib");
    let mysql_lib = basedir.join("lib/mysql");
    let mut library_paths = vec![lib];
    if mysql_lib.is_dir() {
        library_paths.push(mysql_lib);
    }
    if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
        library_paths.extend(std::env::split_paths(&existing));
    }
    let library_path = std::env::join_paths(library_paths)
        .map_err(|e| Error::Other(format!("invalid MariaDB library path: {e}")))?;
    let status = std::process::Command::new(init)
        .arg(format!("--basedir={}", basedir.display()))
        .arg(format!("--datadir={}", datadir.display()))
        .arg("--auth-root-authentication-method=normal")
        .arg("--skip-test-db")
        .env("LD_LIBRARY_PATH", library_path)
        .status()?;
    if !status.success() {
        // A half-written datadir looks initialized to `datadir_missing`, which
        // would make every later start fail with a confusing server error.
        let _ = std::fs::remove_dir_all(datadir);
        return Err(Error::Other(
            "MariaDB data directory initialization failed".into(),
        ));
    }
    Ok(())
}

pub const MARIADB: &str = "mariadb";
pub const MARIADB_SOCKET: &str = "db.sock";

/// Is a mariadbd binary usable on this system?
pub fn mariadb_available(env: &Env) -> Result<bool> {
    Ok(find_mariadbd(env).is_some())
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

    #[test]
    fn managed_mariadb_binary_is_resolved_without_path_install() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        let basedir = env.paths.runtimes().join("mariadb/11.4.13");
        let bin = basedir.join("bin/mariadbd");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "binary").unwrap();
        assert_eq!(resolve_mariadb(&env), Some((bin, basedir)));
    }

    #[test]
    fn managed_mariadb_prefers_highest_numeric_version() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        for version in ["9.11.2", "11.4.13", "11.4.9"] {
            let bin = env
                .paths
                .runtimes()
                .join(format!("mariadb/{version}/bin/mariadbd"));
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            std::fs::write(&bin, "binary").unwrap();
        }
        let (bin, basedir) = resolve_mariadb(&env).unwrap();
        assert_eq!(basedir, env.paths.runtimes().join("mariadb/11.4.13"));
        assert_eq!(bin, basedir.join("bin/mariadbd"));
    }

    #[test]
    fn failed_init_removes_partial_datadir_before_returning_error() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        let basedir = env.paths.runtimes().join("mariadb/11.4.13");
        std::fs::create_dir_all(basedir.join("bin")).unwrap();
        std::fs::write(basedir.join("bin/mariadbd"), "fake").unwrap();
        std::fs::create_dir_all(basedir.join("scripts")).unwrap();
        let init = basedir.join("scripts/mariadb-install-db");
        // mimic a failed initializer: it creates the schema dir, then bails out
        std::fs::write(
            &init,
            "#!/bin/sh\nd=\"${2#--datadir=}\"\n/bin/mkdir -p \"$d/mysql\"\nexit 1\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&init, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let datadir = env.paths.data().join("mariadb");
        let err = init_datadir(&env, &basedir, &datadir).unwrap_err();
        assert!(matches!(err, crate::error::Error::Other(_)), "{err:?}");
        assert!(
            datadir_missing(&datadir),
            "partial datadir must be removed so the next start retries init"
        );
    }
}

/// Generate caddyfile with explicit leaf-certs, falling back to `tls internal`.
pub fn tls_caddyfile_paths(env: &Env, projects: &[String]) -> Vec<String> {
    let certs_dir = env.paths.etc().join("certs");
    projects
        .iter()
        .map(|p| certs_dir.join(format!("{p}.crt")).display().to_string())
        .collect()
}

/// /etc/hosts snippet for the given project names, using the .test TLD.
pub fn hosts_entries(ip: &str, projects: &[String]) -> String {
    let mut lines = Vec::new();
    for p in projects {
        lines.push(format!("{ip} {}.test", p));
    }
    lines.join("\n")
}

#[cfg(test)]
mod hosts_tests {
    use super::*;

    #[test]
    fn hosts_entry_lists_each_project() {
        let out = hosts_entries("127.0.0.1", &["blog".into(), "sub".into()]);
        assert_eq!(out, "127.0.0.1 blog.test\n127.0.0.1 sub.test");
    }

    #[test]
    fn hosts_entry_empty_projects() {
        assert_eq!(hosts_entries("10.0.0.2", &[]), "");
    }
}

/// Find hostile-orphan pids: processes whose cmdline references one of our
/// generated config files (so shutdown-less ghosts can be reaped).
pub fn find_owners(proc_dir: &Path, needles: &[String], skip: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(proc_dir) {
        for e in rd.flatten() {
            if let Ok(n) = e.file_name().to_string_lossy().parse::<u32>() {
                if skip.contains(&n) {
                    continue;
                }
                if let Ok(c) = std::fs::read_to_string(e.path().join("cmdline")) {
                    if !needles.iter().any(|need| c.contains(need)) {
                        continue;
                    }
                    if c.split('\0').all(|t| t != "kill") {
                        out.push(n);
                    }
                }
            }
        }
    }
    out
}

/// Kill (SIGKILL) all ghost processes owning our configs before start.
pub fn reap_owners(needles: &[String], skip: &[u32]) -> usize {
    let mut killed = 0;
    for pid in find_owners(Path::new("/proc"), needles, skip) {
        if std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .output()
            .is_ok()
        {
            killed += 1;
        }
    }
    killed
}

#[cfg(test)]
mod owners_tests {
    use super::*;

    #[test]
    fn find_owners_matches_cmdline_and_ignores_others() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        // pid dir 100: cmdline containing needle
        std::fs::create_dir(p.join("100")).unwrap();
        std::fs::write(
            p.join("100/cmdline"),
            "caddy\0run\0--config\0/etc/Caddyfile\0",
        )
        .unwrap();
        // pid dir 101: unrelated
        std::fs::create_dir(p.join("101")).unwrap();
        std::fs::write(p.join("101/cmdline"), b"sleep\x001000\x00").unwrap();
        // pid dir 102: the reaper itself
        std::fs::create_dir(p.join("102")).unwrap();
        std::fs::write(p.join("102/cmdline"), "kill\0-9\0").unwrap();
        let found = find_owners(p, &["/etc/Caddyfile".to_string()], &[]);
        assert_eq!(found, vec![100]);
        assert!(find_owners(p, &["void-str".to_string()], &[]).is_empty());
    }

    #[test]
    fn find_owners_skips_pids_recorded_in_live_pid_files() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        for pid in [100, 101] {
            std::fs::create_dir(p.join(pid.to_string())).unwrap();
            std::fs::write(
                p.join(pid.to_string()).join("cmdline"),
                "caddy\0run\0--config\0/etc/Caddyfile\0",
            )
            .unwrap();
        }

        let found = find_owners(p, &["/etc/Caddyfile".to_string()], &[100]);

        assert_eq!(
            found,
            vec![101],
            "the pid from a live pid file must not be reaped"
        );
    }
}

/// Build the command specs: (name, bin, args, log) for every service row.
type CmdSpec = (String, String, Vec<String>, std::path::PathBuf);

#[allow(clippy::type_complexity)]
fn service_cmds(env: &Env, ports: &PortPlan, php_version: &str) -> Result<Vec<CmdSpec>> {
    let mut cmds = Vec::new();
    let web = web_server(env);
    let server_bin = resolve_web_bin(env, &web)?;
    cmds.push((
        web.clone(),
        server_bin.display().to_string(),
        web_args(env, &web),
        env.paths.logs().join(format!("{web}.log")),
    ));
    if let Some(php_bin) = php_binary_for(env, php_version) {
        let mut fpm_args = vec![
            "-F".to_string(),
            "-y".to_string(),
            env.fpm_conf_path(php_version).display().to_string(),
        ];
        let ini_path = env.php_ini_path(php_version);
        if ini_path.exists() {
            fpm_args.push("-c".to_string());
            fpm_args.push(ini_path.display().to_string());
        }
        cmds.push((
            format!("php-{php_version}"),
            php_bin.display().to_string(),
            fpm_args,
            env.paths.logs().join(format!("php-{php_version}.log")),
        ));
    }
    // mariadb (system binary) when available
    if mariadb_available(env).unwrap_or(false) {
        let mariadb_cnf = env.paths.etc().join("my.cnf");
        let db_data = env.paths.data().join("mariadb");
        std::fs::create_dir_all(&db_data)?;
        let cmd = mariadb_config(
            env,
            &mariadb_cnf,
            &db_data,
            &env.paths.pids(),
            ports.mariadb,
        )?
        .ok_or_else(|| Error::Other("MariaDB binary was not resolved".into()))?;
        cmds.push((
            MARIADB.to_string(),
            cmd[0].clone(),
            cmd[1..].to_vec(),
            env.paths.logs().join("mariadb.log"),
        ));
    }
    Ok(cmds)
}
