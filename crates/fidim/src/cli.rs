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
        let rp = |k: &str, d: u16| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        CliEnv {
            env: Env::new(root, sites),
            ports: PortPlan {
                http: rp("FIDIM_HTTP_PORT", 80),
                https: rp("FIDIM_HTTPS_PORT", 443),
            },
            php_default: std::env::var("FIDIM_PHP_DEFAULT").unwrap_or_else(|_| "8.4".into()),
        }
    }
    /// Real layout: data under $HOME/.local/share/fine-ill-do-it-myself,
    /// projects under $HOME/Sites.
    pub fn real() -> Result<Self> {
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| fine_ill_do_it_myself::Error::Other("set $HOME".into()))?;
        let rp = |k: &str, d: u16| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        Ok(CliEnv {
            env: Env::new(
                home.join(".local/share/fine-ill-do-it-myself"),
                home.join("Sites"),
            ),
            ports: PortPlan {
                http: rp("FIDIM_HTTP_PORT", 80),
                https: rp("FIDIM_HTTPS_PORT", 443),
            },
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
    let lock = Lock::load(&cli.env.lock_path())?;
    Ok(PhpManager.installed(&lock, &cli.env.paths.bin()))
}

/// Set the default PHP version used by `fidim start`.
pub fn php_use(cli: &CliEnv, version: &str) -> Result<()> {
    if !cli
        .env
        .paths
        .bin()
        .join(format!("php-fpm-{version}"))
        .exists()
    {
        return Err(fine_ill_do_it_myself::Error::PhpNotInstalled(
            version.into(),
        ));
    }
    let mut lock = Lock::load(&cli.env.lock_path())?;
    lock.php_default = Some(version.into());
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

/// Run the unix-socket daemon until terminated.
pub fn daemon(env: &Env, sock: &std::path::Path) -> Result<()> {
    crate::daemon::run_daemon(env.clone(), sock)
}
