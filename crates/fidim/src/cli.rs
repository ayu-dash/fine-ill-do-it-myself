//! Non-interactive subcommands, thin orchestration over `core`.
use fine_ill_do_it_myself::error::Result;
use fine_ill_do_it_myself::manifest::{Downloader, Lock, Manifest, PhpManager};
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
    pub fn with_data_root(root: std::path::PathBuf) -> Self {
        CliEnv {
            env: Env::new(root.join("app"), root.join("Sites")),
            ports: PortPlan::default(),
            php_default: std::env::var("FIDIM_PHP_DEFAULT").unwrap_or_else(|_| "8.4".into()),
        }
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

/// Install a php version from the manifest (download static build).
pub fn php_install(cli: &CliEnv, version: &str) -> Result<()> {
    let manifest = Manifest::load(&cli.env.manifest_path())?;
    let mut lock = Lock::load(&cli.env.lock_path())?;
    let src = manifest
        .find(&format!("php-fpm-{version}"))
        .ok_or_else(|| fine_ill_do_it_myself::Error::MissingBinary(version.to_string()))?
        .clone();
    let bin = cli.env.paths.bin();
    std::fs::create_dir_all(&bin)?;
    Downloader.download(&src, &bin.join(format!("php-{version}")))?;
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

#[cfg(test)]
mod tests {}
