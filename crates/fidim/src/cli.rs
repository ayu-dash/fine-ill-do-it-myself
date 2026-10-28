use fine_ill_do_it_myself::error::{Error, Result};
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
        let sites = if root
            .parent()
            .map(|p| p.join("sites").is_dir())
            .unwrap_or(false)
        {
            root.parent().unwrap().join("sites")
        } else {
            root.join("sites")
        };
        let env = Env::new(root, sites);
        let ports = PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
        let php_default = std::env::var("FIDIM_PHP_DEFAULT").unwrap_or_else(|_| {
            Lock::load(&env.lock_path())
                .ok()
                .and_then(|l| l.php_default)
                .unwrap_or_else(|| "8.4".into())
        });
        CliEnv {
            env,
            ports,
            php_default,
        }
    }
    /// Project layout: everything under <project_root>/.fidim,
    /// keeping all binaries, configs, and sites strictly within the project.
    pub fn real() -> Result<Self> {
        let env = Env::load_real()?;
        let ports = PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
        let php_default = std::env::var("FIDIM_PHP_DEFAULT").unwrap_or_else(|_| {
            Lock::load(&env.lock_path())
                .ok()
                .and_then(|l| l.php_default)
                .unwrap_or_else(|| "8.4".into())
        });
        Ok(CliEnv {
            env,
            ports,
            php_default,
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
/// Shell exports for `eval "$(fidim env)"`: puts managed clients on PATH and
/// points mysql/mariadb at this data root.
pub fn shell_env(cli: &CliEnv) -> String {
    fine_ill_do_it_myself::shell::exports(&cli.env, &cli.ports)
}

/// Where shims go: `$FIDIM_SHIM_DIR`, else `~/.local/bin` (usually on PATH).
pub fn shim_dir() -> Result<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("FIDIM_SHIM_DIR") {
        return Ok(std::path::PathBuf::from(dir));
    }
    let home =
        std::env::var_os("HOME").ok_or_else(|| Error::Other("cannot resolve $HOME".into()))?;
    Ok(std::path::PathBuf::from(home).join(".local/bin"))
}

/// Write mysql/mariadb/php wrappers so they work from any directory.
pub fn install_shims(cli: &CliEnv) -> Result<Vec<std::path::PathBuf>> {
    let shims = fine_ill_do_it_myself::shell::shims(&cli.env, &cli.ports);
    Ok(fine_ill_do_it_myself::shell::install_shims(
        &shim_dir()?,
        &shims,
    )?)
}

pub fn install(cli: &CliEnv, name: &str) -> Result<std::path::PathBuf> {
    cli.env.ensure_dirs()?;
    let manifest = load_or_default_manifest(cli)?;
    let mut lock = Lock::load(&cli.env.lock_path())?;
    let fetcher = fine_ill_do_it_myself::installer::CurlFetcher;
    let out =
        fine_ill_do_it_myself::installer::install(&cli.env, &manifest, &mut lock, name, &fetcher)?;
    if name == fine_ill_do_it_myself::phpmyadmin::SITE {
        // Expose the freshly extracted tree as a site and refresh vhosts so it
        // is reachable right away. Config generation is best-effort: installing
        // phpMyAdmin before PHP should not fail.
        fine_ill_do_it_myself::phpmyadmin::ensure_site(&cli.env, &cli.ports)?;
        let _ = orchestrator::generate_configs(&cli.env, &cli.ports, &cli.php_default);
    }
    Ok(out)
}

pub fn load_or_default_manifest(cli: &CliEnv) -> Result<Manifest> {
    let path = cli.env.manifest_path();
    let defaults = fine_ill_do_it_myself::installer_defaults();
    if !path.exists() {
        std::fs::write(&path, defaults)?;
        return Manifest::load(&path).map_err(fine_ill_do_it_myself::Error::Io);
    }

    let mut manifest = Manifest::load(&path).map_err(fine_ill_do_it_myself::Error::Io)?;
    let bundled = Manifest::parse(defaults)?;
    for source in bundled.binaries {
        if !manifest
            .binaries
            .iter()
            .any(|existing| existing.name == source.name)
        {
            manifest.binaries.push(source);
        }
    }
    Ok(manifest)
}

/// Install a php version from the manifest (download static build).
pub fn php_install(cli: &CliEnv, version: &str) -> Result<()> {
    install(cli, &format!("php-fpm-{version}"))?;
    let manifest = load_or_default_manifest(cli)?;
    if manifest.find(&format!("php-{version}")).is_some() {
        let _ = install(cli, &format!("php-{version}"));
    }
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

/// Install a language version from the manifest.
pub fn lang_install(cli: &CliEnv, lang: &str, version: &str) -> Result<()> {
    if lang == "php" {
        return php_install(cli, version);
    }
    let name = format!("{lang}-{version}");
    install(cli, &name)?;
    let mut lock = Lock::load(&cli.env.lock_path())?;
    let list = lock.languages.entry(lang.to_string()).or_default();
    if !list.contains(&version.to_string()) {
        list.push(version.to_string());
        list.sort();
    }
    lock.save(&cli.env.lock_path())?;
    Ok(())
}

/// List installed versions for a language.
pub fn lang_list(cli: &CliEnv, lang: &str) -> Result<Vec<String>> {
    if lang == "php" {
        return php_list(cli);
    }
    Ok(fine_ill_do_it_myself::lang::installed_lang_versions(
        &cli.env, lang,
    ))
}

#[allow(dead_code)]
fn php_list_old(cli: &CliEnv) -> Result<Vec<String>> {
    let lock = Lock::load(&cli.env.lock_path())?;
    Ok(PhpManager.installed(&lock, &cli.env.paths.bin()))
}

/// Update a symlink at `dest` to point at `src` (removes old link first).
#[cfg(unix)]
fn relink(src: &std::path::Path, dest: &std::path::Path) {
    use std::os::unix::fs::symlink;
    let _ = std::fs::remove_file(dest);
    let _ = symlink(src, dest);
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

    let ini_path = cli.env.php_ini_path(version);
    if let Some(parent) = ini_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let socket = cli
        .env
        .paths
        .pids()
        .join(fine_ill_do_it_myself::orchestrator::MARIADB_SOCKET);
    let _ = std::fs::write(
        &ini_path,
        fine_ill_do_it_myself::php::fpm::generate_php_ini(&[], Some(&socket)),
    );

    let php_dir = cli.env.paths.bin().join("php");
    if php_dir.is_dir() {
        // Update bin/php/php, bin/php/php-fpm, and bin/php/php.ini symlinks (used by CLI and services)
        let src_cli = php_dir.join(format!("php-{version}"));
        let src_fpm = php_dir.join(format!("php-fpm-{version}"));
        #[cfg(unix)]
        {
            if src_cli.exists() {
                relink(&src_cli, &php_dir.join("php"));
            }
            if src_fpm.exists() {
                relink(&src_fpm, &php_dir.join("php-fpm"));
            }
            if ini_path.exists() {
                relink(&ini_path, &php_dir.join("php.ini"));
            }
        }
    }

    let ports =
        fine_ill_do_it_myself::PortPlan::resolve(&cli.env.paths.etc(), |k| std::env::var(k).ok());
    let _ = fine_ill_do_it_myself::orchestrator::generate_configs(&cli.env, &ports, version);
    Ok(())
}

/// Return the path to the php.ini configuration file for the given or default PHP version.
pub fn php_ini(cli: &CliEnv, version: Option<&str>) -> Result<std::path::PathBuf> {
    let ver = match version {
        Some(v) => v.to_string(),
        None => {
            let lock = Lock::load(&cli.env.lock_path()).unwrap_or_default();
            fine_ill_do_it_myself::orchestrator::default_php(&lock)
        }
    };
    let ini = cli.env.php_ini_path(&ver);
    if !ini.exists() {
        let socket = cli
            .env
            .paths
            .pids()
            .join(fine_ill_do_it_myself::orchestrator::MARIADB_SOCKET);
        let _ = std::fs::write(
            &ini,
            fine_ill_do_it_myself::php::fpm::generate_php_ini(&[], Some(&socket)),
        );
    }
    Ok(ini)
}

/// Switch active version for a programming language and update symlinks in bin/<lang>.
pub fn lang_use(cli: &CliEnv, lang: &str, version: &str) -> Result<()> {
    if lang == "php" {
        return php_use(cli, version);
    }
    let clean_version = if version.starts_with("system") {
        "system"
    } else {
        version
    };
    let src =
        fine_ill_do_it_myself::lang::lang_binary_for(&cli.env, lang, version).ok_or_else(|| {
            fine_ill_do_it_myself::Error::Other(format!("{lang} {version} not installed"))
        })?;

    let mut lock = Lock::load(&cli.env.lock_path())?;
    lock.language_defaults
        .insert(lang.to_string(), clean_version.to_string());
    lock.save(&cli.env.lock_path())?;

    let lang_dir = cli.env.paths.bin().join(lang);
    let _ = std::fs::create_dir_all(&lang_dir);

    #[cfg(unix)]
    {
        // `python` installs expose `python-<version>` but users type `python3`;
        // link both names to the selected binary.
        let mains: &[&str] = match lang {
            "python" => &["python", "python3"],
            other => &[other],
        };
        for name in mains {
            relink(&src, &lang_dir.join(name));
        }

        // (link name, source stem). `pip3` shares the `pip-<version>` binary.
        let companions: &[(&str, &str)] = match lang {
            "node" => &[("npm", "npm"), ("npx", "npx"), ("corepack", "corepack")],
            "python" => &[("pip", "pip"), ("pip3", "pip")],
            "go" => &[("gofmt", "gofmt")],
            "java" => &[("javac", "javac"), ("jar", "jar")],
            _ => &[],
        };

        // Resolve the real runtime bin dir. `src` is usually a symlink in
        // bin/<lang>/ (e.g. node-24 -> runtimes/node/24/bin/node), so looking
        // next to the symlink would find the wrong file (or itself).
        let runtime_bin = src
            .canonicalize()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));

        for &(link_name, source_stem) in companions {
            let link_dest = lang_dir.join(link_name);
            let mut comp_src = None;
            if clean_version != "system" {
                let managed_comp = lang_dir.join(format!("{source_stem}-{clean_version}"));
                if managed_comp.exists() {
                    comp_src = Some(managed_comp);
                }
            }
            if comp_src.is_none() {
                if let Some(dir) = &runtime_bin {
                    for name in [source_stem, link_name] {
                        let beside = dir.join(name);
                        // never link a name to itself when `src` is not a symlink
                        if beside.exists() && beside != link_dest {
                            comp_src = Some(beside);
                            break;
                        }
                    }
                }
            }
            if let Some(comp_path) = comp_src {
                relink(&comp_path, &link_dest);
            }
        }
    }
    Ok(())
}

/// Remove an installed language version.
pub fn lang_remove(cli: &CliEnv, lang: &str, version: &str) -> Result<()> {
    if lang == "php" {
        return php_remove(cli, version);
    }
    let installed = fine_ill_do_it_myself::lang::installed_lang_versions(&cli.env, lang);
    if !installed.iter().any(|v| v == version) {
        return Err(Error::Other(format!("{lang} {version} is not installed")));
    }
    let mut lock = Lock::load(&cli.env.lock_path())?;
    if let Some(list) = lock.languages.get_mut(lang) {
        list.retain(|v| v != version);
    }
    let was_default = lock.language_defaults.get(lang).map(|s| s.as_str()) == Some(version);
    if was_default {
        lock.language_defaults.remove(lang);
    }
    lock.save(&cli.env.lock_path())?;

    let lang_dir = cli.env.paths.bin().join(lang);
    let _ = std::fs::remove_file(lang_dir.join(format!("{lang}-{version}")));
    let _ = std::fs::remove_file(cli.env.paths.bin().join(format!("{lang}-{version}")));
    // The extracted runtime tree (and a drop-in folder) would otherwise leak
    // hundreds of megabytes and keep the version listed as installed.
    let _ = std::fs::remove_dir_all(cli.env.paths.runtimes().join(lang).join(version));
    let _ = std::fs::remove_dir_all(cli.env.paths.root().join(lang).join(version));

    if was_default {
        let next = fine_ill_do_it_myself::lang::installed_lang_versions(&cli.env, lang)
            .into_iter()
            .find(|v| v != "system");
        match next {
            Some(next) => lang_use(cli, lang, &next)?,
            None => {
                #[cfg(unix)]
                remove_lang_links(&lang_dir, lang);
            }
        }
    }
    Ok(())
}

/// Drop the `bin/<lang>/` entry points so removing the last installed version
/// never leaves a symlink pointing at a deleted runtime.
#[cfg(unix)]
fn remove_lang_links(lang_dir: &std::path::Path, lang: &str) {
    let mains: &[&str] = match lang {
        "python" => &["python", "python3"],
        other => &[other],
    };
    let companions: &[&str] = match lang {
        "node" => &["npm", "npx", "corepack"],
        "python" => &["pip", "pip3"],
        "go" => &["gofmt"],
        "java" => &["javac", "jar"],
        _ => &[],
    };
    for name in mains.iter().chain(companions.iter()) {
        let path = lang_dir.join(name);
        let is_link = std::fs::symlink_metadata(&path)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if is_link {
            let _ = std::fs::remove_file(&path);
        }
    }
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

/// A project created by `fidim new` / the TUI quick-create flow.
#[derive(Debug, Clone)]
pub struct CreatedProject {
    pub path: std::path::PathBuf,
    pub template: fine_ill_do_it_myself::project::Template,
    /// Database created alongside the project (`--db`).
    pub database: Option<String>,
}

/// Create a project, optionally its database, and refresh the vhosts.
pub fn create_project_with<F, C>(
    cli: &CliEnv,
    name: &str,
    template: fine_ill_do_it_myself::project::Template,
    with_db: bool,
    fetcher: &F,
    runner: &C,
) -> Result<CreatedProject>
where
    F: fine_ill_do_it_myself::installer::Fetcher,
    C: fine_ill_do_it_myself::proc::CommandRunner,
{
    let path = fine_ill_do_it_myself::project::create(&cli.env, name, template, fetcher, runner)?;
    let database = if with_db {
        match fine_ill_do_it_myself::database::create_database(&cli.env, name, runner) {
            Ok(()) => Some(name.to_string()),
            Err(e) => {
                // `new --db` must not leave a project whose database failed.
                let _ = std::fs::remove_dir_all(&path);
                return Err(e);
            }
        }
    } else {
        None
    };
    // Refresh vhosts/certs/hosts so `<name>.test` is live right away.
    orchestrator::generate_configs(&cli.env, &cli.ports, &cli.php_default)?;
    Ok(CreatedProject {
        path,
        template,
        database,
    })
}

/// `fidim new`: create the project with the real downloader/runner, then
/// restart the active web server only when it was already running.
pub fn create_project(
    cli: &CliEnv,
    name: &str,
    template: fine_ill_do_it_myself::project::Template,
    with_db: bool,
) -> Result<CreatedProject> {
    let fetcher = fine_ill_do_it_myself::installer::CurlFetcher;
    let runner = fine_ill_do_it_myself::proc::RealCommandRunner;
    let made = create_project_with(cli, name, template, with_db, &fetcher, &runner)?;
    restart_web_if_running(cli)?;
    Ok(made)
}

/// New vhosts only take effect after a reload; stop/start the chosen server
/// when it is up, and stay silent when it is not.
fn restart_web_if_running(cli: &CliEnv) -> Result<()> {
    let lock = Lock::load(&cli.env.lock_path()).unwrap_or_default();
    let web = orchestrator::default_web(&lock);
    let running = orchestrator::status_all(&cli.env, &cli.ports, &RealRunner)
        .iter()
        .any(|s| s.name == web && s.state == fine_ill_do_it_myself::service::ServiceState::Running);
    if !running {
        return Ok(());
    }
    orchestrator::stop_service(&cli.env, &cli.ports, &RealRunner, &web)?;
    orchestrator::start_service(&cli.env, &cli.ports, &RealRunner, &cli.php_default, &web)?;
    Ok(())
}

/// Uninstall a php version.
pub fn php_remove(cli: &CliEnv, version: &str) -> Result<()> {
    if fine_ill_do_it_myself::orchestrator::php_binary_for(&cli.env, version).is_none() {
        return Err(fine_ill_do_it_myself::Error::PhpNotInstalled(
            version.into(),
        ));
    }
    let mut lock = Lock::load(&cli.env.lock_path())?;
    lock.php.retain(|v| v != version);
    let was_default = lock.php_default.as_deref() == Some(version);
    if was_default {
        lock.php_default = lock.php.first().cloned();
    }
    let next = lock.php_default.clone();
    lock.save(&cli.env.lock_path())?;
    let _ = std::fs::remove_file(
        cli.env
            .paths
            .bin()
            .join("php")
            .join(format!("php-fpm-{version}")),
    );
    let _ = std::fs::remove_file(
        cli.env
            .paths
            .bin()
            .join("php")
            .join(format!("php-{version}")),
    );
    let _ = std::fs::remove_file(cli.env.paths.bin().join(format!("php-fpm-{version}")));
    let _ = std::fs::remove_file(cli.env.paths.bin().join(format!("php-{version}")));
    let _ = std::fs::remove_dir_all(cli.env.php_etc_dir(version));
    // Keep bin/php/{php,php-fpm,php.ini} pointing at a version that exists.
    if was_default {
        let php_dir = cli.env.paths.bin().join("php");
        match next {
            Some(next) => php_use(cli, &next)?,
            None =>
            {
                #[cfg(unix)]
                for link in ["php", "php-fpm", "php.ini"] {
                    let path = php_dir.join(link);
                    let is_link = std::fs::symlink_metadata(&path)
                        .map(|meta| meta.file_type().is_symlink())
                        .unwrap_or(false);
                    if is_link {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }
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

/// Write the missing entries to /etc/hosts using sudo tee -a.
pub fn hosts_sync(entries: &[String]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut payload = entries.join("\n");
    payload.push('\n');
    let mut child = std::process::Command::new("sudo")
        .args(["tee", "-a", "/etc/hosts"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::Other(format!("failed to spawn sudo: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(payload.as_bytes())
            .map_err(|e| Error::Other(format!("failed to write to sudo: {e}")))?;
    }
    let status = child
        .wait()
        .map_err(|e| Error::Other(format!("failed to wait for sudo: {e}")))?;
    if !status.success() {
        return Err(Error::Other("sudo tee -a /etc/hosts failed".into()));
    }
    Ok(())
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

fn home_dir() -> Result<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| Error::Other("cannot resolve $HOME".into()))
}

fn dns_plan() -> Result<fine_ill_do_it_myself::dns::InstallPlan> {
    let exe = std::env::current_exe()
        .map_err(|e| Error::Other(format!("cannot locate the fidim binary: {e}")))?;
    Ok(fine_ill_do_it_myself::dns::install_plan(
        &exe,
        &home_dir()?,
        fine_ill_do_it_myself::dns::DEFAULT_PORT,
    ))
}

fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .map_err(|e| Error::Other(format!("failed to run {program}: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "{program} {} failed ({status})",
            args.join(" ")
        )))
    }
}

fn run_sudo_tee(path: &std::path::Path, body: &str) -> Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("sudo")
        .args(["tee", &path.display().to_string()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::Other(format!("failed to spawn sudo: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(body.as_bytes())
            .map_err(|e| Error::Other(format!("failed to write to sudo: {e}")))?;
    }
    let status = child
        .wait()
        .map_err(|e| Error::Other(format!("sudo tee failed: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other("sudo tee was rejected".into()))
    }
}

fn systemd_resolved_active() -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", "systemd-resolved"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Report whether seamless `*.test` resolution is installed and running.
pub fn dns_status() -> Result<Vec<String>> {
    let plan = dns_plan()?;
    let port = fine_ill_do_it_myself::dns::DEFAULT_PORT;
    let active = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "fidim-dns.service"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    Ok(vec![
        format!("resolver: 127.0.0.1:{port}"),
        format!(
            "resolved drop-in: {} ({})",
            plan.dropin.display(),
            if plan.dropin.exists() {
                "present"
            } else {
                "missing"
            }
        ),
        format!(
            "user unit: {} ({})",
            plan.unit.display(),
            if plan.unit.exists() {
                "present"
            } else {
                "missing"
            }
        ),
        format!("service: {active}"),
    ])
}

/// One-time setup that makes every `*.test` name resolve to 127.0.0.1.
///
/// Without `apply` it only prints the plan and the root commands. With
/// `apply` it writes a per-user systemd unit (no root) and, via sudo, a
/// systemd-resolved drop-in that forwards only `.test`.
pub fn dns_install(apply: bool) -> Result<()> {
    let plan = dns_plan()?;
    let port = fine_ill_do_it_myself::dns::DEFAULT_PORT;
    println!("fidim can answer every *.test name itself, so new projects never need /etc/hosts.");
    println!("This installs:");
    println!(
        "  {} (routes *.test to 127.0.0.1:{port})",
        plan.dropin.display()
    );
    println!(
        "  {} (keeps the responder running in your session)",
        plan.unit.display()
    );

    if !apply {
        println!("\nNothing was changed. Apply with:\n  fidim dns install --yes");
        println!("\nRoot is used once, for:");
        for line in fine_ill_do_it_myself::dns::root_steps(&plan) {
            println!("{line}");
        }
        return Ok(());
    }

    if !systemd_resolved_active() {
        return Err(Error::Other(
            "systemd-resolved is not running; use `fidim hosts` as a fallback".into(),
        ));
    }

    if let Some(parent) = plan.unit.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plan.unit, &plan.unit_body)?;
    run_command("systemctl", &["--user", "daemon-reload"])?;
    run_command(
        "systemctl",
        &["--user", "enable", "--now", "fidim-dns.service"],
    )?;

    let sudo = run_command("sudo", &["mkdir", "-p", "/etc/systemd/resolved.conf.d"])
        .and_then(|()| run_sudo_tee(&plan.dropin, &plan.dropin_body))
        .and_then(|()| run_command("sudo", &["systemctl", "restart", "systemd-resolved"]));
    if let Err(e) = sudo {
        eprintln!("\nsudo failed: {e}");
        eprintln!("The responder is installed and running, but *.test is not routed to it yet.");
        eprintln!(
            "Run this in a terminal that can prompt for a password, then `fidim dns status`:\n"
        );
        for line in fine_ill_do_it_myself::dns::root_steps(&plan) {
            eprintln!("{line}\n");
        }
        return Err(Error::Other(
            "resolved drop-in not installed; see the commands above".into(),
        ));
    }

    println!("\ninstalled: any *.test name now resolves to 127.0.0.1");
    Ok(())
}

/// Undo [`dns_install`].
pub fn dns_uninstall(apply: bool) -> Result<()> {
    let plan = dns_plan()?;
    if !apply {
        println!("This removes:");
        println!("  {}", plan.dropin.display());
        println!("  {}", plan.unit.display());
        println!("\nNothing was changed. Apply with:\n  fidim dns uninstall --yes");
        return Ok(());
    }
    let _ = run_command(
        "systemctl",
        &["--user", "disable", "--now", "fidim-dns.service"],
    );
    let _ = std::fs::remove_file(&plan.unit);
    let _ = run_command("systemctl", &["--user", "daemon-reload"]);
    let dropin = plan.dropin.display().to_string();
    run_command("sudo", &["rm", "-f", &dropin])?;
    run_command("sudo", &["systemctl", "restart", "systemd-resolved"])?;
    println!("removed: *.test uses the system resolver again");
    Ok(())
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
    fn lang_use_python_links_python3_and_pip3_to_selected_version() {
        use std::os::unix::fs::symlink;

        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());
        let lang_dir = cli.env.paths.bin().join("python");
        std::fs::create_dir_all(&lang_dir).unwrap();
        for f in ["python-3.13", "pip-3.13"] {
            std::fs::write(lang_dir.join(f), "").unwrap();
        }
        // stale links from a previous setup pointing at the system runtime
        symlink("/usr/bin/python3", lang_dir.join("python3")).unwrap();
        symlink("/usr/bin/pip3", lang_dir.join("pip3")).unwrap();

        lang_use(&cli, "python", "3.13").unwrap();

        for (link, want) in [
            ("python", "python-3.13"),
            ("python3", "python-3.13"),
            ("pip", "pip-3.13"),
            ("pip3", "pip-3.13"),
        ] {
            let got = std::fs::read_link(lang_dir.join(link)).unwrap();
            assert_eq!(
                got.file_name().unwrap(),
                want,
                "{link} -> {}",
                got.display()
            );
        }
    }

    #[test]
    fn lang_use_links_node_and_java_companions_from_the_runtime() {
        use std::os::unix::fs::symlink;

        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());
        for (lang, version, tools) in [
            ("node", "24", &["node", "npm", "npx", "corepack"][..]),
            ("java", "25", &["java", "javac", "jar"][..]),
        ] {
            let rt = cli
                .env
                .paths
                .runtimes()
                .join(lang)
                .join(version)
                .join("bin");
            std::fs::create_dir_all(&rt).unwrap();
            for tool in tools {
                std::fs::write(rt.join(tool), "").unwrap();
            }
            let lang_dir = cli.env.paths.bin().join(lang);
            std::fs::create_dir_all(&lang_dir).unwrap();
            symlink(
                rt.join(tools[0]),
                lang_dir.join(format!("{lang}-{version}")),
            )
            .unwrap();
            // the last tool (corepack/jar) is not in the manifest's versioned files
            for tool in &tools[1..tools.len() - 1] {
                symlink(rt.join(tool), lang_dir.join(format!("{tool}-{version}"))).unwrap();
            }

            lang_use(&cli, lang, version).unwrap();

            let got = std::fs::read_link(lang_dir.join(lang)).unwrap();
            assert_eq!(
                got.file_name().unwrap().to_string_lossy(),
                format!("{lang}-{version}")
            );

            // corepack/jar are not in the manifest's versioned files, so they
            // must be resolved from the extracted runtime instead.
            let comp = if lang == "node" { "corepack" } else { "jar" };
            let got = std::fs::read_link(lang_dir.join(comp)).unwrap();
            assert_eq!(got, rt.join(comp), "{lang} {comp}");
        }
    }

    #[test]
    fn lang_remove_deletes_the_runtime_and_relinks_the_remaining_version() {
        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());
        let lang_dir = cli.env.paths.bin().join("python");
        std::fs::create_dir_all(&lang_dir).unwrap();
        for v in ["3.11", "3.12"] {
            std::fs::write(lang_dir.join(format!("python-{v}")), "").unwrap();
            std::fs::create_dir_all(cli.env.paths.runtimes().join("python").join(v)).unwrap();
        }
        lang_use(&cli, "python", "3.11").unwrap();

        lang_remove(&cli, "python", "3.11").unwrap();

        assert!(
            !cli.env.paths.runtimes().join("python/3.11").exists(),
            "the extracted runtime must be removed with the version"
        );
        let link = std::fs::read_link(lang_dir.join("python3")).unwrap();
        assert_eq!(
            link.file_name().unwrap(),
            "python-3.12",
            "the active link must move to a version that still exists"
        );
    }

    #[test]
    fn lang_remove_rejects_a_version_that_is_not_installed() {
        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());

        let err = lang_remove(&cli, "node", "99").unwrap_err();

        assert!(format!("{err}").contains("not installed"), "{err}");
    }

    #[test]
    fn php_remove_rejects_a_version_that_is_not_installed() {
        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());

        let err = php_remove(&cli, "9.9").unwrap_err();

        assert!(format!("{err}").contains("not installed"), "{err}");
    }

    struct FailingRunner;

    impl fine_ill_do_it_myself::proc::CommandRunner for FailingRunner {
        fn run(&self, _cmd: &[String], _envs: &[(String, String)]) -> Result<String> {
            Err(Error::Other("database server is down".into()))
        }
    }

    #[test]
    fn new_with_db_rolls_back_the_project_when_the_database_fails() {
        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());
        let client = cli.env.paths.bin().join("mariadb/mariadb");
        std::fs::create_dir_all(client.parent().unwrap()).unwrap();
        std::fs::write(&client, "").unwrap();

        let err = create_project_with(
            &cli,
            "shop",
            fine_ill_do_it_myself::project::Template::Blank,
            true,
            &NoFetcher,
            &FailingRunner,
        )
        .unwrap_err();

        assert!(format!("{err}").contains("database"), "{err}");
        assert!(
            !cli.env.paths.sites().join("shop").exists(),
            "a failed --db must not leave a half project behind"
        );
    }

    #[test]
    fn port_set_rejects_unknown_keys() {
        let t = tempfile::tempdir().unwrap();
        let err = port_set(t.path(), "banana", 1).unwrap_err();
        assert!(format!("{err}").contains("unknown port key"), "{err}");
    }

    #[derive(Default)]
    struct NoFetcher;

    impl fine_ill_do_it_myself::installer::Fetcher for NoFetcher {
        fn fetch(&self, url: &str) -> Result<Vec<u8>> {
            Err(Error::Other(format!("no network in tests: {url}")))
        }
    }

    #[derive(Default)]
    struct RecordingRunner {
        commands: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    }

    impl fine_ill_do_it_myself::proc::CommandRunner for RecordingRunner {
        fn run(&self, cmd: &[String], _envs: &[(String, String)]) -> Result<String> {
            self.commands.lock().unwrap().push(cmd.to_vec());
            Ok(String::new())
        }
    }

    #[test]
    fn create_project_scaffolds_and_regenerates_the_vhost() {
        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());

        let made = create_project_with(
            &cli,
            "blog",
            fine_ill_do_it_myself::project::Template::Php,
            false,
            &NoFetcher,
            &RecordingRunner::default(),
        )
        .unwrap();

        assert_eq!(made.path, cli.env.paths.sites().join("blog"));
        assert!(made.path.join("index.php").is_file());
        assert_eq!(made.database, None);
        let caddy = std::fs::read_to_string(cli.env.caddyfile_path()).unwrap();
        assert!(caddy.contains("blog.test"), "{caddy}");
    }

    #[test]
    fn create_project_regenerates_a_reverse_proxy_vhost_for_node() {
        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());

        create_project_with(
            &cli,
            "app",
            fine_ill_do_it_myself::project::Template::Node,
            false,
            &NoFetcher,
            &RecordingRunner::default(),
        )
        .unwrap();

        let caddy = std::fs::read_to_string(cli.env.caddyfile_path()).unwrap();
        assert!(caddy.contains("reverse_proxy 127.0.0.1:3000"), "{caddy}");
    }

    #[test]
    fn create_project_with_db_creates_the_database() {
        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());
        let client = cli.env.paths.bin().join("mariadb/mariadb");
        std::fs::create_dir_all(client.parent().unwrap()).unwrap();
        std::fs::write(&client, "").unwrap();
        let runner = RecordingRunner::default();

        let made = create_project_with(
            &cli,
            "shop",
            fine_ill_do_it_myself::project::Template::Blank,
            true,
            &NoFetcher,
            &runner,
        )
        .unwrap();

        assert_eq!(made.database.as_deref(), Some("shop"));
        let commands = runner.commands.lock().unwrap();
        assert!(
            commands.iter().any(|cmd| cmd
                .iter()
                .any(|a| a.contains("CREATE DATABASE IF NOT EXISTS `shop`"))),
            "{commands:?}"
        );
    }

    #[test]
    fn create_project_without_db_never_touches_mariadb() {
        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());
        let runner = RecordingRunner::default();

        create_project_with(
            &cli,
            "blog",
            fine_ill_do_it_myself::project::Template::Php,
            false,
            &NoFetcher,
            &runner,
        )
        .unwrap();

        assert!(runner.commands.lock().unwrap().is_empty());
    }

    #[test]
    fn create_project_rejects_an_invalid_name_before_writing_anything() {
        let t = tempfile::tempdir().unwrap();
        let cli = CliEnv::with_data_root(t.path().to_path_buf());

        let err = create_project_with(
            &cli,
            "Bad_Name",
            fine_ill_do_it_myself::project::Template::Php,
            false,
            &NoFetcher,
            &RecordingRunner::default(),
        )
        .unwrap_err();

        assert!(format!("{err}").contains("a-z"), "{err}");
        assert!(!cli.env.paths.sites().join("Bad_Name").exists());
    }
}
