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
    /// Print shell exports so managed clients (mysql, php, node, python, go, java) work here
    Env {
        /// Install wrappers into ~/.local/bin (or $FIDIM_SHIM_DIR) instead
        #[arg(long)]
        install: bool,
    },
    /// Manage /etc/hosts entries for .test projects (fallback when DNS is unavailable)
    Hosts {
        /// Only print missing entries without modifying /etc/hosts
        #[arg(long, short = 'n')]
        dry_run: bool,
        /// Deprecated alias (syncing is now default)
        #[arg(long, short, hide = true)]
        sync: bool,
    },
    /// Seamless *.test resolution via a wildcard local DNS responder
    Dns {
        #[command(subcommand)]
        cmd: DnsCommands,
    },
    /// Print steps to trust the local CA (needs root)
    Trust,
    /// Run the background daemon exposing a unix socket
    Daemon,
    Projects,
    /// Create a project under Sites from a template
    New {
        /// Project name, or template followed by a project name
        #[arg(value_name = "NAME_OR_TEMPLATE", num_args = 1..=2)]
        args: Vec<String>,
        /// laravel, wordpress, php, html, blank, node or python
        #[arg(long, short)]
        template: Option<String>,
        /// Also create a MariaDB database with the same name
        #[arg(long)]
        db: bool,
    },
    /// Download and install a managed binary (caddy, php-fpm-8.4, ...)
    Install {
        name: String,
    },
    /// Manage PHP versions
    Php {
        #[command(subcommand)]
        cmd: PhpCommands,
    },
    /// Manage Node.js versions
    Node {
        #[command(subcommand)]
        cmd: LangCommands,
    },
    /// Manage Python versions
    Python {
        #[command(subcommand)]
        cmd: LangCommands,
    },
    /// Manage Go versions
    Go {
        #[command(subcommand)]
        cmd: LangCommands,
    },
    /// Manage Java versions
    Java {
        #[command(subcommand)]
        cmd: LangCommands,
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
enum DnsCommands {
    /// Run the wildcard DNS responder in the foreground
    Serve {
        #[arg(long, default_value_t = fine_ill_do_it_myself::dns::DEFAULT_PORT)]
        port: u16,
    },
    /// Show whether *.test resolution is installed and running
    Status,
    /// One-time setup: route *.test to fidim (prints the plan unless --yes)
    Install {
        #[arg(long)]
        yes: bool,
    },
    /// Remove the resolved drop-in and the user unit
    Uninstall {
        #[arg(long)]
        yes: bool,
    },
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
    /// Show the path to the php.ini configuration file
    Ini {
        version: Option<String>,
    },
}

#[derive(Subcommand)]
enum LangCommands {
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
        Some(Commands::Env { install }) => {
            if install {
                let written = cli::install_shims(&env)?;
                if written.is_empty() {
                    println!(
                        "no managed clients found; run `fidim install mariadb` or `fidim php install <version>` first"
                    );
                } else {
                    for p in written {
                        println!("{}", p.display());
                    }
                }
            } else {
                print!("{}", cli::shell_env(&env));
            }
        }
        Some(Commands::Hosts { dry_run, sync: _ }) => {
            match cli::hosts_missing(&env) {
                Ok(entries) => {
                    if entries.is_empty() {
                        println!("hosts up to date");
                    } else if dry_run {
                        for e in &entries {
                            println!("{e}");
                        }
                        let all = entries.join("\n");
                        println!("\nTo apply, run:\n  fidim hosts\nor:\n  sudo tee -a /etc/hosts <<'EOF'\n{all}\nEOF");
                    } else {
                        for e in &entries {
                            println!("adding to /etc/hosts: {e}");
                        }
                        cli::hosts_sync(&entries)?;
                        println!(
                            "successfully updated /etc/hosts (added {} entries)",
                            entries.len()
                        );
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }?;
        }
        Some(Commands::Dns { cmd }) => match cmd {
            DnsCommands::Serve { port } => fine_ill_do_it_myself::dns::serve(port)?,
            DnsCommands::Status => {
                for line in cli::dns_status()? {
                    println!("{line}");
                }
            }
            DnsCommands::Install { yes } => cli::dns_install(yes)?,
            DnsCommands::Uninstall { yes } => cli::dns_uninstall(yes)?,
        },
        Some(Commands::Trust) => {
            for line in cli::trust(&env)? {
                println!("{line}");
            }
        }
        Some(Commands::Install { name }) => {
            let p = cli::install(&env, &name)?;
            println!("installed {name} -> {}", p.display());
            if name == fine_ill_do_it_myself::phpmyadmin::SITE {
                println!(
                    "phpMyAdmin: {}",
                    fine_ill_do_it_myself::phpmyadmin::url(&env.env, &env.ports)
                );
                println!(
                    "login as root with an empty password (add the host once with: fidim hosts)"
                );
            }
        }
        Some(Commands::Projects) => {
            for p in cli::projects(&env)? {
                println!("{p}: http://{p}.test");
            }
        }
        Some(Commands::New { args, template, db }) => {
            let (name, template) = match (args.as_slice(), template) {
                ([name], Some(template)) => (name.clone(), template),
                ([name], None) => (name.clone(), "php".into()),
                ([template, name], None) => (name.clone(), template.clone()),
                ([_, _], Some(_)) => {
                    return Err(fine_ill_do_it_myself::Error::Other(
                        "use either `fidim new <name> --template <template>` or `fidim new <template> <name>`, not both".into(),
                    )
                    .into());
                }
                _ => {
                    return Err(fine_ill_do_it_myself::Error::Other(
                        "provide a project name, optionally preceded by a template".into(),
                    )
                    .into());
                }
            };
            let template = fine_ill_do_it_myself::project::Template::parse(&template).ok_or_else(
                || {
                    fine_ill_do_it_myself::Error::Other(format!(
                        "unknown template `{template}`; use laravel, wordpress, php, html, blank, node or python"
                    ))
                },
            )?;
            let made = cli::create_project(&env, &name, template, db)?;
            println!(
                "created {name} ({}) at {}",
                made.template.id(),
                made.path.display()
            );
            println!("{name}: http://{name}.test");
            if let Some(database) = &made.database {
                println!("database: {database}");
            }
            let next = template.next_step(&made.path);
            if !next.is_empty() {
                println!("next: {next}");
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
            PhpCommands::Ini { version } => {
                let path = cli::php_ini(&env, version.as_deref())?;
                println!("{}", path.display());
            }
        },
        Some(Commands::Node { cmd }) => handle_lang_cmd(&env, "node", cmd)?,
        Some(Commands::Python { cmd }) => handle_lang_cmd(&env, "python", cmd)?,
        Some(Commands::Go { cmd }) => handle_lang_cmd(&env, "go", cmd)?,
        Some(Commands::Java { cmd }) => handle_lang_cmd(&env, "java", cmd)?,
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

fn handle_lang_cmd(
    env: &cli::CliEnv,
    lang: &str,
    cmd: LangCommands,
) -> Result<(), fine_ill_do_it_myself::Error> {
    match cmd {
        LangCommands::Install { version } => {
            cli::lang_install(env, lang, &version)?;
            println!("{lang} {version} installed");
        }
        LangCommands::List => {
            for v in cli::lang_list(env, lang)? {
                println!("{v}");
            }
        }
        LangCommands::Use { version } => {
            cli::lang_use(env, lang, &version)?;
            println!("default {lang} {version}");
        }
        LangCommands::Remove { version } => {
            cli::lang_remove(env, lang, &version)?;
            println!("{lang} {version} removed");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    #[test]
    fn env_command_prints_shell_exports() {
        let t = tempfile::tempdir().unwrap();
        let mut cmd = Command::cargo_bin("fidim").unwrap();
        let out = cmd
            .arg("--data-root")
            .arg(t.path())
            .arg("env")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("export MYSQL_UNIX_PORT='"), "{s}");
        assert!(s.contains("export MYSQL_TCP_PORT=3306"), "{s}");
    }

    #[test]
    fn env_install_writes_client_shims() {
        let data = tempfile::tempdir().unwrap();
        let shims = tempfile::tempdir().unwrap();
        let clients = data.path().join("bin/mariadb");
        std::fs::create_dir_all(&clients).unwrap();
        for tool in ["mysql", "mariadb"] {
            std::fs::write(clients.join(tool), "").unwrap();
        }
        let php = data.path().join("bin/php");
        std::fs::create_dir_all(&php).unwrap();
        std::fs::write(php.join("php"), "").unwrap();
        let node = data.path().join("bin/node");
        std::fs::create_dir_all(&node).unwrap();
        for tool in ["node", "npm", "npx"] {
            std::fs::write(node.join(tool), "").unwrap();
        }

        let out = Command::cargo_bin("fidim")
            .unwrap()
            .env("FIDIM_SHIM_DIR", shims.path())
            .arg("--data-root")
            .arg(data.path())
            .args(["env", "--install"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        for name in ["mysql", "mariadb", "php", "node", "npm", "npx"] {
            assert!(shims.path().join(name).exists(), "{name} shim missing");
        }
        let body = std::fs::read_to_string(shims.path().join("mysql")).unwrap();
        assert!(body.contains("MYSQL_UNIX_PORT='"), "{body}");
        let node = std::fs::read_to_string(shims.path().join("node")).unwrap();
        assert!(!node.contains("MYSQL_UNIX_PORT"), "{node}");
    }

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
    fn dns_install_prints_the_plan_without_touching_the_system() {
        let out = Command::cargo_bin("fidim")
            .unwrap()
            .args(["dns", "install"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("resolved.conf.d/90-fidim.conf"), "{s}");
        assert!(s.contains("sudo systemctl restart systemd-resolved"), "{s}");
        assert!(s.contains("--yes"), "must explain how to apply\n{s}");
    }

    #[test]
    fn projects_command_prints_nothing_for_empty_projects() {
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

    #[test]
    fn new_command_scaffolds_a_php_project_and_prints_its_url() {
        let t = tempfile::tempdir().unwrap();
        let out = Command::cargo_bin("fidim")
            .unwrap()
            .env("HOME", t.path())
            .arg("--data-root")
            .arg(t.path())
            .args(["new", "blog"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let sites = t.path().join("projects");
        assert!(sites.join("blog/index.php").is_file());
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("blog"), "{s}");
        assert!(s.contains("blog.test"), "{s}");
    }

    #[test]
    fn new_command_accepts_template_before_project_name() {
        let t = tempfile::tempdir().unwrap();
        let out = Command::cargo_bin("fidim")
            .unwrap()
            .env("HOME", t.path())
            .arg("--data-root")
            .arg(t.path())
            .args(["new", "html", "landing"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(t.path().join("projects/landing/index.html").is_file());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("created landing (html)"), "{stdout}");
    }

    #[test]
    fn new_command_rejects_an_unknown_template() {
        let t = tempfile::tempdir().unwrap();
        let out = Command::cargo_bin("fidim")
            .unwrap()
            .env("HOME", t.path())
            .arg("--data-root")
            .arg(t.path())
            .args(["new", "blog", "--template", "django"])
            .output()
            .unwrap();
        assert!(!out.status.success());
        let s = String::from_utf8_lossy(&out.stderr);
        assert!(s.contains("unknown template"), "{s}");
        assert!(
            s.contains("wordpress"),
            "must list the valid templates: {s}"
        );
    }
}
mod daemon;
mod tui;
