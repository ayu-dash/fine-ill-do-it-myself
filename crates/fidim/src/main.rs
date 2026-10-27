use anyhow::Result as AResult;
use clap::{Parser, Subcommand};

mod cli;

#[derive(Parser)]
#[command(
    name = "fidim",
    about = "fine ill do it myself - local web dev stack",
    version
)]
struct Cli {
    #[arg(long, help = "use an alternative data root instead of $HOME")]
    data_root: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Open the interactive TUI
    Tui,
    Start {
        /// Start a single service instead of everything
        service: Option<String>,
    },
    Stop {
        /// Stop a single service instead of everything
        service: Option<String>,
    },
    Restart,
    Status,
    /// Manage /etc/hosts entries for .test projects
    Hosts,
    /// Print steps to trust the local CA (needs root)
    Trust,
    /// Run the background daemon exposing a unix socket
    Daemon,
    Projects,
    /// Download and install a managed binary (caddy, php-fpm-8.4, ...)
    Install {
        name: String,
    },
    /// Manage PHP versions
    Php {
        #[command(subcommand)]
        cmd: PhpCommands,
    },
    /// Show or change port assignments (etc/ports.toml)
    Ports {
        #[command(subcommand)]
        cmd: PortCommands,
    },
    /// Choose the web server: caddy, apache (httpd) or nginx
    Web {
        #[command(subcommand)]
        cmd: WebCommands,
    },
}

#[derive(Subcommand)]
enum PortCommands {
    /// Show current ports (shared base + each web server's own pair)
    Show,
    /// Set a port key: http, https, fpm_base, mariadb, or a per-server
    /// pair like caddy_http / caddy_https / httpd_http / nginx_https / ...
    Set { key: String, value: u16 },
}

#[derive(Subcommand)]
enum WebCommands {
    /// Use one of: caddy, apache, nginx
    Use { server: String },
    /// Show currently selected web server
    Show,
}

#[derive(Subcommand)]
enum PhpCommands {
    Install {
        version: String,
    },
    List,
    #[clap(name = "use")]
    Use {
        version: String,
    },
    Remove {
        version: String,
    },
}

fn resolve(cli: &Cli) -> cli::CliEnv {
    match &cli.data_root {
        Some(root) => cli::CliEnv::with_data_root(root.clone()),
        None => cli::CliEnv::real().expect("cannot resolve $HOME"),
    }
}

fn main() {
    let cli = Cli::parse();
    let env = resolve(&cli);
    match run(cli, env) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}

fn run(cli: Cli, env: cli::CliEnv) -> AResult<()> {
    match cli.command {
        Some(Commands::Start { service }) => match service.as_deref() {
            None => {
                for s in cli::start_all(&env)? {
                    println!("{s}");
                }
            }
            Some(name) => {
                let s = cli::start_service(&env, name)?;
                println!("{}: {:?}", s.name, s.state);
            }
        },
        Some(Commands::Stop { service }) => match service.as_deref() {
            None => {
                cli::stop_all(&env)?;
                println!("stopped");
            }
            Some(name) => {
                cli::stop_service(&env, name)?;
                println!("{name} stopped");
            }
        },
        Some(Commands::Restart) => {
            cli::stop_all(&env)?;
            for s in cli::start_all(&env)? {
                println!("{s}");
            }
        }
        Some(Commands::Status) => {
            for s in cli::status(&env)? {
                println!("{s}");
            }
        }
        Some(Commands::Hosts) => {
            match cli::hosts_missing(&env) {
                Ok(entries) => {
                    if entries.is_empty() {
                        println!("hosts up to date");
                    } else {
                        for e in &entries {
                            println!("{e}");
                        }
                        let all = entries.join("\n");
                        println!("sudo tee -a /etc/hosts <<'EOF'\n{all}\nEOF");
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }?;
        }
        Some(Commands::Trust) => {
            for line in cli::trust(&env)? {
                println!("{line}");
            }
        }
        Some(Commands::Install { name }) => {
            let p = cli::install(&env, &name)?;
            println!("installed {name} -> {}", p.display());
        }
        Some(Commands::Projects) => {
            for p in cli::projects(&env)? {
                println!("{p}: http://{p}.test");
            }
        }
        Some(Commands::Php { cmd }) => match cmd {
            PhpCommands::Install { version } => {
                cli::php_install(&env, &version)?;
                println!("php {version} installed");
            }
            PhpCommands::List => {
                for v in cli::php_list(&env)? {
                    println!("{v}");
                }
            }
            PhpCommands::Use { version } => {
                cli::php_use(&env, &version)?;
                println!("default php {version}");
            }
            PhpCommands::Remove { version } => {
                cli::php_remove(&env, &version)?;
                println!("php {version} removed");
            }
        },
        Some(Commands::Ports { cmd }) => match cmd {
            PortCommands::Show => {
                let p = &env.ports;
                println!(
                    "http = {p}\nhttps = {p2}\nfpm_base = {p3}\nmariadb = {p4}\
                     \ncaddy    = http {p5} https {p6}\
                     \nhttpd    = http {p7} https {p8}\
                     \nnginx    = http {p9} https {p10}\
                     \nlighttpd = http {p11} https {p12}",
                    p = p.http,
                    p2 = p.https,
                    p3 = p.fpm_base,
                    p4 = p.mariadb,
                    p5 = p.caddy_http,
                    p6 = p.caddy_https,
                    p7 = p.httpd_http,
                    p8 = p.httpd_https,
                    p9 = p.nginx_http,
                    p10 = p.nginx_https,
                    p11 = p.lighttpd_http,
                    p12 = p.lighttpd_https
                );
            }
            PortCommands::Set { key, value } => {
                let etc = env.env.paths.etc();
                cli::port_set(&etc, &key, value)?;
                println!("{key} = {value}");
            }
        },
        Some(Commands::Web { cmd }) => match cmd {
            WebCommands::Use { server } => {
                cli::web_use(&env, &server)?;
                println!("web server: {server}");
            }
            WebCommands::Show => {
                let lock = fine_ill_do_it_myself::Lock::load(&env.env.lock_path())?;
                println!(
                    "{}",
                    fine_ill_do_it_myself::orchestrator::default_web(&lock)
                );
            }
        },
        Some(Commands::Daemon) => {
            let sock = env.env.paths.data().join("fidim.sock");
            cli::daemon(&env.env, &sock)?;
        }
        Some(Commands::Tui) | None => {
            tui::run_tui(&env.env, env.ports)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    #[test]
    fn ports_show_lists_every_web_servers_own_ports() {
        let t = tempfile::tempdir().unwrap();
        let mut cmd = Command::cargo_bin("fidim").unwrap();
        let out = cmd
            .arg("--data-root")
            .arg(t.path())
            .args(["ports", "show"])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("caddy    = http 8080 https 8443"), "{s}");
        assert!(s.contains("nginx    = http 8082 https 8445"), "{s}");
    }

    #[test]
    fn ports_set_accepts_per_web_server_key_and_show_picks_it_up() {
        let t = tempfile::tempdir().unwrap();
        let mut cmd = Command::cargo_bin("fidim").unwrap();
        let out = cmd
            .arg("--data-root")
            .arg(t.path())
            .args(["ports", "set", "nginx_http", "9090"])
            .output()
            .unwrap();
        assert!(out.status.success(), "set failed");
        let mut cmd = Command::cargo_bin("fidim").unwrap();
        let out = cmd
            .arg("--data-root")
            .arg(t.path())
            .args(["ports", "show"])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("nginx    = http 9090"), "{s}");
    }

    #[test]
    fn status_reports_stopped_with_alternate_data_root() {
        let t = tempfile::tempdir().unwrap();
        let mut cmd = Command::cargo_bin("fidim").unwrap();
        let out = cmd
            .arg("--data-root")
            .arg(t.path())
            .arg("status")
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("caddy: Stopped"), "stdout: {s}");
    }

    #[test]
    fn projects_command_prints_nothing_for_empty_sites() {
        let t = tempfile::tempdir().unwrap();
        let mut cmd = Command::cargo_bin("fidim").unwrap();
        let out = cmd
            .env("HOME", t.path())
            .arg("--data-root")
            .arg(t.path())
            .arg("projects")
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    }
}
mod daemon;
mod tui;
