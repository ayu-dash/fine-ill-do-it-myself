//! Non-interactive subcommands, thin orchestration over `core`.
use fine_ill_do_it_myself::error::Result;
use fine_ill_do_it_myself::manifest::{Lock, Manifest, PhpManager};
use fine_ill_do_it_myself::orchestrator;
use fine_ill_do_it_myself::proc::RealRunner;
use fine_ill_do_it_myself::supervisor::PortPlan;
use fine_ill_do_it_myself::Env;

/// Live handle wrapping the Env for real invocations. `with_data_root` exists
/// for tests: point everything at a temporary directory.
pub struct CliEnv {
    pub env: Env,
    pub ports: PortPlan,
    pub php_default: String,
}

impl CliEnv {
    /// Test helper: everything under the given temporary root.
    pub fn with_data_root(root: std::path::PathBuf) -> Self {
        let sites = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join("Sites");
        let env = Env::new(root, sites);
        let ports = PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
        CliEnv {
            env,
            ports,
            php_default: std::env::var("FIDIM_PHP_DEFAULT").unwrap_or_else(|_| "8.4".into()),
        }
    }
    /// Real layout: data under $HOME/.local/share/fine-ill-do-it-myself,
    /// projects under $HOME/Sites.
    pub fn real() -> Result<Self> {
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| fine_ill_do_it_myself::Error::Other("set $HOME".into()))?;
        let env = Env::new(
            home.join(".local/share/fine-ill-do-it-myself"),
            home.join("Sites"),
        );
        let ports = PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
        Ok(CliEnv {
            env,
            ports,
            php_default: std::env::var("FIDIM_PHP_DEFAULT").unwrap_or_else(|_| "8.4".into()),
        })
    }
}

/// Start caddy + default php-fpm (idempotent).
pub fn start_all(cli: &CliEnv) -> Result<Vec<String>> {
    let runner = RealRunner;
    Ok(
        orchestrator::start_all(&cli.env, &cli.ports, &runner, &cli.php_default)?
            .iter()
            .map(|s| format!("{}: {:?}", s.name, s.state))
            .collect(),
    )
}

/// Start one named service (php-<ver>, mariadb, or a web server).
pub fn start_service(
    cli: &CliEnv,
    name: &str,
) -> Result<fine_ill_do_it_myself::service::ServiceStatus> {
    orchestrator::start_service(&cli.env, &cli.ports, &RealRunner, &cli.php_default, name)
}

/// Stop one named service (php-<ver>, mariadb, or a web server).
pub fn stop_service(cli: &CliEnv, name: &str) -> Result<()> {
    orchestrator::stop_service(&cli.env, &cli.ports, &RealRunner, name)
}

pub fn stop_all(cli: &CliEnv) -> Result<()> {
    orchestrator::stop_all(&cli.env, &cli.ports, &RealRunner)
}

pub fn status(cli: &CliEnv) -> Result<Vec<String>> {
    Ok(orchestrator::status_all(&cli.env, &cli.ports, &RealRunner)
        .iter()
        .map(|s| format!("{}: {:?}", s.name, s.state))
        .collect())
}

pub fn projects(cli: &CliEnv) -> Result<Vec<String>> {
    Ok(orchestrator::list_projects(&cli.env))
}

/// Download + install a binary by name (caddy, php-fpm-8.4, ...).
pub fn install(cli: &CliEnv, name: &str) -> Result<std::path::PathBuf> {
    cli.env.ensure_dirs()?;
    let manifest = load_or_default_manifest(cli)?;
    let mut lock = Lock::load(&cli.env.lock_path())?;
    let fetcher = fine_ill_do_it_myself::installer::HttpFetcher;
    let out =
        fine_ill_do_it_myself::installer::install(&cli.env, &manifest, &mut lock, name, &fetcher)?;
    Ok(out)
}

pub fn load_or_default_manifest(cli: &CliEnv) -> Result<Manifest> {
    let path = cli.env.manifest_path();
    if !path.exists() {
        std::fs::write(&path, fine_ill_do_it_myself::installer_defaults())?;
    }
    Manifest::load(&path).map_err(fine_ill_do_it_myself::Error::Io)
}

/// Install a php version from the manifest (download static build).
pub fn php_install(cli: &CliEnv, version: &str) -> Result<()> {
    install(cli, &format!("php-fpm-{version}"))?;
    let mut lock = Lock::load(&cli.env.lock_path())?;
    lock.php.push(version.to_string());
    lock.php.sort();
    lock.php.dedup();
    lock.save(&cli.env.lock_path())?;
    Ok(())
}

pub fn php_list(cli: &CliEnv) -> Result<Vec<String>> {
    let mut out = PhpManager.installed(&Lock::load(&cli.env.lock_path())?, &cli.env.paths.bin());
    for v in PhpManager.dropped_in(&cli.env.paths.root().join("php")) {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

#[allow(dead_code)]
fn php_list_old(cli: &CliEnv) -> Result<Vec<String>> {
    let lock = Lock::load(&cli.env.lock_path())?;
    Ok(PhpManager.installed(&lock, &cli.env.paths.bin()))
}

/// Set the default PHP version used by `fidim start`.
pub fn php_use(cli: &CliEnv, version: &str) -> Result<()> {
    if fine_ill_do_it_myself::orchestrator::php_binary_for(&cli.env, version).is_none() {
        return Err(fine_ill_do_it_myself::Error::PhpNotInstalled(
            version.into(),
        ));
    }
    let mut lock = Lock::load(&cli.env.lock_path())?;
    lock.php_default = Some(version.into());
    lock.save(&cli.env.lock_path())?;
    Ok(())
}

/// Set one port key in `etc/ports.toml`. Common keys (http, https, fpm_base,
/// mariadb) plus one pair per web server: caddy_http(s), httpd_http(s),
/// nginx_http(s), lighttpd_http(s) — each server keeps its own ports.
pub fn port_set(etc: &std::path::Path, key: &str, value: u16) -> Result<()> {
    let allowed = [
        "http",
        "https",
        "fpm_base",
        "mariadb",
        "caddy_http",
        "caddy_https",
        "httpd_http",
        "httpd_https",
        "nginx_http",
        "nginx_https",
        "lighttpd_http",
        "lighttpd_https",
    ];
    if !allowed.contains(&key) {
        return Err(fine_ill_do_it_myself::Error::Other(format!(
            "unknown port key {key}; use http, https, fpm_base, mariadb, or a per-server pair like caddy_http / caddy_https / httpd_http / httpd_https / nginx_http / nginx_https / lighttpd_http / lighttpd_https"
        )));
    }
    std::fs::create_dir_all(etc)?;
    let path = etc.join("ports.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with(&format!("{key} =")))
        .map(|l| l.to_string())
        .collect();
    lines.push(format!("{key} = {value}"));
    std::fs::write(&path, lines.join("\n") + "\n")?;
    Ok(())
}

/// Persist the chosen web server into lock.toml.
pub fn web_use(cli: &CliEnv, server: &str) -> Result<()> {
    let name = match server {
        "caddy" => fine_ill_do_it_myself::orchestrator::CADDY,
        "apache" | "httpd" => fine_ill_do_it_myself::orchestrator::APACHE,
        "nginx" => fine_ill_do_it_myself::orchestrator::NGINX,
        "lighttpd" => fine_ill_do_it_myself::orchestrator::LIGHTTPD,
        other => {
            return Err(fine_ill_do_it_myself::Error::Other(format!(
                "unknown web server {other}; use caddy, apache, nginx or lighttpd"
            )))
        }
    };
    let mut lock = Lock::load(&cli.env.lock_path())?;
    lock.web_default = Some(name.to_string());
    lock.save(&cli.env.lock_path())?;
    Ok(())
}

/// Uninstall a php version.
pub fn php_remove(cli: &CliEnv, version: &str) -> Result<()> {
    let mut lock = Lock::load(&cli.env.lock_path())?;
    lock.php.retain(|v| v != version);
    if lock.php_default.as_deref() == Some(version) {
        lock.php_default = lock.php.first().cloned();
    }
    lock.save(&cli.env.lock_path())?;
    let _ = std::fs::remove_file(cli.env.paths.bin().join(format!("php-fpm-{version}")));
    let _ = std::fs::remove_file(cli.env.paths.bin().join(format!("php-{version}")));
    Ok(())
}

/// Compose the missing /etc/hosts lines for ".test" projects.
pub fn hosts_missing(cli: &CliEnv) -> Result<Vec<String>> {
    let projects = fine_ill_do_it_myself::list_projects(&cli.env);
    let hosts = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
    Ok(projects
        .into_iter()
        .filter(|p| !hosts.contains(&format!("{p}.{HOST_TLD}")))
        .map(|p| format!("127.0.0.1 {p}.{HOST_TLD}"))
        .collect())
}

const HOST_TLD: &str = "test";

/// Print the (root) steps to trust the fidim local CA system-wide.
pub fn trust(cli: &CliEnv) -> Result<Vec<String>> {
    let ca = cli.env.paths.etc().join("ca.crt");
    Ok(vec![
        "sudo mkdir -p /usr/local/share/ca-certificates".to_string(),
        format!(
            "sudo cp {} /usr/local/share/ca-certificates/fidim-ca.crt",
            ca.display()
        ),
        "sudo update-ca-certificates".into(),
        "apt install libnss3-tools   # browsers (NSS)".into(),
        format!(
            "certutil -d sql:$HOME/.pki/nssdb -A -t 'CT,c,c' -n fidim-ca -i {path}",
            path = ca.display()
        ),
    ])
}

/// Run the unix-socket daemon until terminated.
pub fn daemon(env: &Env, sock: &std::path::Path) -> Result<()> {
    crate::daemon::run_daemon(env.clone(), sock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_set_accepts_per_web_server_keys() {
        let t = tempfile::tempdir().unwrap();
        port_set(t.path(), "caddy_http", 9090).unwrap();
        port_set(t.path(), "nginx_https", 9443).unwrap();
        port_set(t.path(), "lighttpd_http", 9080).unwrap();
        let text = std::fs::read_to_string(t.path().join("ports.toml")).unwrap();
        assert!(text.contains("caddy_http = 9090"), "{text}");
        assert!(text.contains("nginx_https = 9443"), "{text}");
        assert!(text.contains("lighttpd_http = 9080"), "{text}");
        // setting twice replaces, not appends
        port_set(t.path(), "caddy_http", 9091).unwrap();
        let text = std::fs::read_to_string(t.path().join("ports.toml")).unwrap();
        assert_eq!(text.matches("caddy_http = ").count(), 1, "{text}");
        assert!(text.contains("caddy_http = 9091"), "{text}");
    }

    #[test]
    fn port_set_rejects_unknown_keys() {
        let t = tempfile::tempdir().unwrap();
        let err = port_set(t.path(), "banana", 1).unwrap_err();
        assert!(format!("{err}").contains("unknown port key"), "{err}");
    }
}
