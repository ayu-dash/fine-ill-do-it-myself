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
    Start,
    Stop,
    Restart,
    Status,
    /// Manage /etc/hosts entries for .test projects
    Hosts,
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
        Some(Commands::Start) => {
            for s in cli::start_all(&env)? {
                println!("{s}");
            }
        }
        Some(Commands::Stop) => {
            cli::stop_all(&env)?;
            println!("stopped");
        }
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
        Some(Commands::Tui) | None => {
            tui::run_tui(&env.env)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

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
            .arg("--data-root")
            .arg(t.path())
            .arg("projects")
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    }
}
mod tui;
