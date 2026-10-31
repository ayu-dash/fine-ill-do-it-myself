//! Shell integration: print `export` lines so managed clients (mysql, mariadb,
//! php, node, ...) can be used directly from the user's terminal, and write
//! tiny shims into a directory that is already on `PATH` (usually
//! `~/.local/bin`) so the commands also work from any other directory.

use std::path::{Path, PathBuf};

use crate::supervisor::{shlex_quote, Env, PortPlan};

const SHELL_START_MARKER: &str = "# >>> fidim shell integration >>>";
const SHELL_END_MARKER: &str = "# <<< fidim shell integration <<<";

/// Managed client binaries that get a shim when they exist under `bin/mariadb/`.
pub const SHIM_TOOLS: &[&str] = &[
    "mysql",
    "mariadb",
    "mysqladmin",
    "mariadb-admin",
    "mysqldump",
    "mariadb-dump",
];

/// Managed language binaries that get a shim when they exist under `bin/<lang>/`.
/// The `<lang>` entry is the symlink updated by `fidim <lang> use <version>`.
pub const LANGUAGE_SHIMS: &[(&str, &[&str])] = &[
    ("node", &["node", "npm", "npx", "corepack"]),
    ("python", &["python", "python3", "pip", "pip3"]),
    ("go", &["go", "gofmt"]),
    ("java", &["java", "javac", "jar"]),
];

/// Directories under `bin/` that are prepended to `PATH`.
const PATH_DIRS: &[&str] = &["mariadb", "php", "node", "python", "go", "java"];

/// Render `export ...` lines for this data root. Meant to be evaluated:
/// `eval "$(fidim env)"`.
pub fn exports(env: &Env, ports: &PortPlan) -> String {
    let mut out = String::new();
    let mut dirs = Vec::new();
    for name in PATH_DIRS {
        let dir = env.paths.bin().join(name);
        if dir.is_dir() {
            dirs.push(shlex_quote(&dir.display().to_string()));
        }
    }
    // composer.phar lives directly in bin/ (single file, not a per-tool dir)
    if crate::composer::is_installed(env) {
        dirs.push(shlex_quote(&env.paths.bin().display().to_string()));
    }
    if !dirs.is_empty() {
        out.push_str(&format!("export PATH={}:\"$PATH\"\n", dirs.join(":")));
    }
    // default socket for mysql/mariadb clients (MYSQL_UNIX_PORT is honoured by
    // both libmariadb and libmysqlclient)
    let socket = env.paths.pids().join(crate::orchestrator::MARIADB_SOCKET);
    out.push_str(&format!(
        "export MYSQL_UNIX_PORT={}\n",
        shlex_quote(&socket.display().to_string())
    ));
    out.push_str(&format!("export MYSQL_TCP_PORT={}\n", ports.mariadb));
    out
}

/// Find the startup file for the currently supported POSIX shells.
pub fn shell_rc_path(home: &Path, shell: &str) -> std::io::Result<PathBuf> {
    let name = Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let rc = match name {
        "bash" => ".bashrc",
        "zsh" => ".zshrc",
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("persistent shell integration is not supported for {name:?}"),
            ));
        }
    };
    Ok(home.join(rc))
}

/// Add a backed-up, idempotent PATH integration block to a user's shell rc file.
pub fn install_shell_integration(
    rc_path: &Path,
    shim_dirs: &[PathBuf],
    data_root: &Path,
) -> std::io::Result<Option<PathBuf>> {
    if shim_dirs.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "at least one shim directory is required",
        ));
    }
    let existing = match std::fs::symlink_metadata(rc_path) {
        Ok(metadata) if metadata.file_type().is_file() => Some(metadata),
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "refusing to modify non-regular shell config {}",
                    rc_path.display()
                ),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let original = match &existing {
        Some(_) => std::fs::read(rc_path)?,
        None => Vec::new(),
    };
    let mut snippet = format!("{SHELL_START_MARKER}\n");
    for shim_dir in shim_dirs {
        let shim = shlex_quote(&shim_dir.display().to_string());
        snippet.push_str(&format!(
            "case \":$PATH:\" in\n  *:{shim}:*) ;;\n  *) export PATH={shim}:\"$PATH\" ;;\nesac\n"
        ));
    }
    snippet.push_str(&format!(
        "eval \"$(fidim --data-root {} env)\"\n{SHELL_END_MARKER}\n",
        shlex_quote(&data_root.display().to_string())
    ));

    let updated = if existing.is_some() {
        let text = std::str::from_utf8(&original)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let starts: Vec<_> = text.match_indices(SHELL_START_MARKER).collect();
        let ends: Vec<_> = text.match_indices(SHELL_END_MARKER).collect();
        match (starts.as_slice(), ends.as_slice()) {
            ([], []) => {
                let mut updated = original.clone();
                if !updated.is_empty() && !updated.ends_with(b"\n") {
                    updated.push(b'\n');
                }
                updated.extend_from_slice(b"\n");
                updated.extend_from_slice(snippet.as_bytes());
                updated
            }
            ([(start, _)], [(end_start, _)]) if start < end_start => {
                let mut end = *end_start + SHELL_END_MARKER.len();
                if text.as_bytes().get(end) == Some(&b'\n') {
                    end += 1;
                }
                if text[*start..end] == snippet {
                    return Ok(None);
                }
                format!("{}{}{}", &text[..*start], snippet, &text[end..]).into_bytes()
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "incomplete or duplicated fidim shell integration block in {}",
                        rc_path.display()
                    ),
                ));
            }
        }
    } else {
        snippet.into_bytes()
    };

    let backup = if existing.is_some() {
        let name = rc_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("shellrc");
        let mut suffix = 0u32;
        loop {
            let backup_name = if suffix == 0 {
                format!("{name}.fidim.bak")
            } else {
                format!("{name}.fidim.bak.{suffix}")
            };
            let backup = rc_path.with_file_name(backup_name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    if let Err(error) = file.write_all(&original).and_then(|()| file.sync_all()) {
                        let _ = std::fs::remove_file(&backup);
                        return Err(error);
                    }
                    break Some(backup);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    suffix = suffix.checked_add(1).ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            "no unused shell config backup name available",
                        )
                    })?;
                }
                Err(error) => return Err(error),
            }
        }
    } else {
        None
    };

    let name = rc_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("shellrc");
    let staged = rc_path.with_file_name(format!(".{name}.fidim.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        use std::io::Write;
        file.write_all(&updated)?;
        file.sync_all()?;
        if let Some(metadata) = &existing {
            std::fs::set_permissions(&staged, metadata.permissions())?;
        }
        std::fs::rename(&staged, rc_path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result.map(|()| backup)
}

/// Wrapper scripts for every managed client that exists, as (name, body).
pub fn shims(env: &Env, ports: &PortPlan) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let clients = env.paths.bin().join("mariadb");
    let socket = env.paths.pids().join(crate::orchestrator::MARIADB_SOCKET);
    for tool in SHIM_TOOLS {
        let target = clients.join(tool);
        if target.is_file() {
            out.push((
                (*tool).to_string(),
                shim_script(&target, Some((&socket, ports.mariadb))),
            ));
        }
    }
    let php = env.paths.bin().join("php").join("php");
    if php.is_file() {
        out.push(("php".to_string(), shim_script(&php, None)));
    }
    if crate::composer::is_installed(env) {
        if let Some(php) = crate::composer::php_binary(env) {
            out.push((
                crate::composer::NAME.to_string(),
                composer_shim(
                    &php,
                    &crate::composer::phar_path(env),
                    &crate::composer::env_vars(env),
                ),
            ));
        }
    }
    for (lang, tools) in LANGUAGE_SHIMS {
        let dir = env.paths.bin().join(lang);
        for tool in *tools {
            let target = dir.join(tool);
            if target.is_file() {
                out.push(((*tool).to_string(), shim_script(&target, None)));
            }
        }
    }
    out
}

/// Write shims into `dir` (created when missing) and make them executable.
pub fn install_shims(dir: &Path, shims: &[(String, String)]) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    for (name, _) in shims {
        let path = dir.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let managed = metadata.file_type().is_file()
                    && std::fs::read_to_string(&path)
                        .is_ok_and(|contents| contents.contains("# generated by fidim;"));
                if !managed {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!(
                            "refusing to replace user-owned command {}; remove it or choose another FIDIM_SHIM_DIR",
                            path.display()
                        ),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }

    let mut written = Vec::new();
    for (index, (name, body)) in shims.iter().enumerate() {
        let path = dir.join(name);
        let staged = dir.join(format!(".{name}.fidim.{}.{}", std::process::id(), index));
        std::fs::write(&staged, body)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&staged, &path)?;
        written.push(path);
    }
    Ok(written)
}

fn shim_script(target: &Path, db: Option<(&Path, u16)>) -> String {
    let mut script = String::from(
        "#!/bin/sh\n# generated by fidim; re-run `fidim env --install` after moving the data root\n",
    );
    if let Some((socket, port)) = db {
        script.push_str(&format!(
            "export MYSQL_UNIX_PORT={}\n",
            shlex_quote(&socket.display().to_string())
        ));
        script.push_str(&format!("export MYSQL_TCP_PORT={port}\n"));
    }
    script.push_str(&format!(
        "exec {} \"$@\"\n",
        shlex_quote(&target.display().to_string())
    ));
    script
}

/// Composer is a phar, so its shim has to name the managed PHP explicitly and
/// keep composer's own state inside the data root.
fn composer_shim(php: &Path, phar: &Path, vars: &[(String, String)]) -> String {
    let mut script = String::from(
        "#!/bin/sh\n# generated by fidim; re-run `fidim env --install` after moving the data root\n",
    );
    for (key, value) in vars {
        script.push_str(&format!("export {key}={}\n", shlex_quote(value)));
    }
    script.push_str(&format!(
        "exec {} {} \"$@\"\n",
        shlex_quote(&php.display().to_string()),
        shlex_quote(&phar.display().to_string())
    ));
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with_root(root: &std::path::Path) -> Env {
        Env::new(root.join("app"), root.join("Sites"))
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn exports_point_path_and_mysql_clients_at_the_data_root() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        std::fs::create_dir_all(env.paths.bin().join("mariadb")).unwrap();
        std::fs::create_dir_all(env.paths.bin().join("php")).unwrap();
        let out = exports(&env, &PortPlan::default());
        let clients = env.paths.bin().join("mariadb").display().to_string();
        let php = env.paths.bin().join("php").display().to_string();
        assert!(out.contains(&format!("'{clients}'")), "{out}");
        assert!(out.contains(&format!("'{php}'")), "{out}");
        assert!(out.contains(":\"$PATH\""), "{out}");
        let sock = env.paths.pids().join("db.sock").display().to_string();
        assert!(
            out.contains(&format!("export MYSQL_UNIX_PORT='{sock}'")),
            "{out}"
        );
        assert!(out.contains("export MYSQL_TCP_PORT=3306"), "{out}");
    }

    #[test]
    fn exports_quote_paths_containing_spaces() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(&t.path().join("dir with space"));
        std::fs::create_dir_all(env.paths.bin().join("mariadb")).unwrap();
        let out = exports(&env, &PortPlan::default());
        let clients = env.paths.bin().join("mariadb").display().to_string();
        assert!(clients.contains(' '), "test needs a path with a space");
        assert!(out.contains(&format!("export PATH='{clients}'")), "{out}");
    }

    #[test]
    fn exports_skip_path_when_no_managed_client_is_installed() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        let out = exports(&env, &PortPlan::default());
        assert!(!out.contains("export PATH="), "{out}");
        assert!(out.contains("export MYSQL_UNIX_PORT='"), "{out}");
    }

    #[test]
    fn exports_use_the_configured_mariadb_port() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        let ports = PortPlan {
            mariadb: 3307,
            ..PortPlan::default()
        };
        assert!(exports(&env, &ports).contains("export MYSQL_TCP_PORT=3307"));
    }

    #[test]
    fn shims_cover_installed_clients_and_php() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        for tool in ["mysql", "mariadb", "mariadb-admin"] {
            touch(&env.paths.bin().join("mariadb").join(tool));
        }
        touch(&env.paths.bin().join("php").join("php"));

        let shims = shims(&env, &PortPlan::default());
        let names: Vec<&str> = shims.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"mysql"), "{names:?}");
        assert!(names.contains(&"mariadb"), "{names:?}");
        assert!(names.contains(&"php"), "{names:?}");
        assert!(
            !names.contains(&"mysqldump"),
            "missing tool must be skipped"
        );

        let mysql = &shims.iter().find(|(n, _)| n == "mysql").unwrap().1;
        assert!(mysql.contains("MYSQL_UNIX_PORT='"), "{mysql}");
        assert!(mysql.contains("exec '"), "{mysql}");
        let php = &shims.iter().find(|(n, _)| n == "php").unwrap().1;
        assert!(!php.contains("MYSQL_UNIX_PORT"), "{php}");
    }

    #[test]
    fn exports_include_installed_language_bin_dirs() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        for lang in ["node", "python", "go", "java"] {
            std::fs::create_dir_all(env.paths.bin().join(lang)).unwrap();
        }
        let out = exports(&env, &PortPlan::default());
        for lang in ["node", "python", "go", "java"] {
            let dir = env.paths.bin().join(lang).display().to_string();
            assert!(
                out.contains(&format!("'{dir}'")),
                "{lang} missing from PATH: {out}"
            );
        }
    }

    #[test]
    fn exports_put_composer_on_path() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        touch(&env.paths.bin().join("php").join("php"));
        touch(&env.paths.bin().join("composer"));

        let out = exports(&env, &PortPlan::default());

        let bin = format!("'{}'", env.paths.bin().display());
        let line = out
            .lines()
            .find(|l| l.starts_with("export PATH="))
            .unwrap_or_default();
        assert!(
            line.contains(&bin),
            "bin dir with composer must be on PATH: {out}"
        );
    }

    #[test]
    fn exports_do_not_add_the_bin_dir_without_composer() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        touch(&env.paths.bin().join("php").join("php"));

        let out = exports(&env, &PortPlan::default());

        let bin = format!("'{}'", env.paths.bin().display());
        let line = out
            .lines()
            .find(|l| l.starts_with("export PATH="))
            .unwrap_or_default();
        assert!(!line.contains(&bin), "{out}");
    }

    #[test]
    fn shims_run_composer_through_the_managed_php() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        touch(&env.paths.bin().join("php").join("php"));
        touch(&env.paths.bin().join("composer"));

        let shims = shims(&env, &PortPlan::default());
        let composer = shims
            .iter()
            .find(|(n, _)| n == "composer")
            .map(|(_, body)| body.clone())
            .expect("composer shim must exist");

        let php = env.paths.bin().join("php/php").display().to_string();
        let phar = env.paths.bin().join("composer").display().to_string();
        assert!(
            composer.contains(&format!("exec '{php}' '{phar}'")),
            "{composer}"
        );
        assert!(composer.contains("COMPOSER_HOME='"), "{composer}");
        assert!(composer.contains("COMPOSER_CACHE_DIR='"), "{composer}");
    }

    #[test]
    fn composer_shim_is_skipped_without_php() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        touch(&env.paths.bin().join("composer"));

        let shims = shims(&env, &PortPlan::default());

        assert!(
            !shims.iter().any(|(n, _)| n == "composer"),
            "composer cannot run without php"
        );
    }

    #[test]
    fn shims_cover_installed_language_binaries() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        for tool in ["node", "npm", "npx"] {
            touch(&env.paths.bin().join("node").join(tool));
        }
        for tool in ["python", "python3", "pip", "pip3"] {
            touch(&env.paths.bin().join("python").join(tool));
        }
        for tool in ["go", "gofmt"] {
            touch(&env.paths.bin().join("go").join(tool));
        }
        for tool in ["java", "javac", "jar"] {
            touch(&env.paths.bin().join("java").join(tool));
        }

        let shims = shims(&env, &PortPlan::default());
        let names: Vec<&str> = shims.iter().map(|(n, _)| n.as_str()).collect();
        for tool in [
            "node", "npm", "npx", "python3", "pip3", "go", "gofmt", "java", "javac", "jar",
        ] {
            assert!(names.contains(&tool), "{tool} missing from {names:?}");
        }
        assert!(
            !names.contains(&"corepack"),
            "missing tool must be skipped: {names:?}"
        );

        let node = &shims.iter().find(|(n, _)| n == "node").unwrap().1;
        assert!(node.contains("exec '"), "{node}");
        assert!(!node.contains("MYSQL_UNIX_PORT"), "{node}");
    }

    #[test]
    fn install_shims_writes_executable_wrappers() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("bin");
        let written =
            install_shims(&dir, &[("mysql".into(), "#!/bin/sh\nexit 0\n".into())]).unwrap();
        assert_eq!(written, vec![dir.join("mysql")]);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&written[0]).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn install_shims_refuses_to_replace_user_commands() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        let command = dir.join("php");
        std::fs::write(&command, "#!/bin/sh\necho user\n").unwrap();

        let error =
            install_shims(&dir, &[("php".into(), "# generated by fidim;\n".into())]).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(command).unwrap(),
            "#!/bin/sh\necho user\n"
        );
    }

    #[test]
    fn install_shims_can_update_existing_fidim_shims() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("php"),
            "#!/bin/sh\n# generated by fidim; old target\n",
        )
        .unwrap();

        install_shims(
            &dir,
            &[("php".into(), "# generated by fidim; new target\n".into())],
        )
        .unwrap();

        assert!(std::fs::read_to_string(dir.join("php"))
            .unwrap()
            .contains("new target"));
    }

    #[test]
    fn shell_rc_path_supports_bash_and_zsh_only() {
        let home = Path::new("/home/test");
        assert_eq!(
            shell_rc_path(home, "/bin/bash").unwrap(),
            home.join(".bashrc")
        );
        assert_eq!(
            shell_rc_path(home, "/usr/bin/zsh").unwrap(),
            home.join(".zshrc")
        );
        assert_eq!(
            shell_rc_path(home, "/usr/bin/fish").unwrap_err().kind(),
            std::io::ErrorKind::Unsupported
        );
    }

    #[test]
    fn shell_integration_backups_and_updates_rc_idempotently() {
        let t = tempfile::tempdir().unwrap();
        let rc = t.path().join(".bashrc");
        let shim_dir = t.path().join("local bin");
        std::fs::write(&rc, "export EDITOR=vim\n").unwrap();

        let backup =
            install_shell_integration(&rc, std::slice::from_ref(&shim_dir), t.path()).unwrap();
        let backup = backup.expect("existing rc file must be backed up");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "export EDITOR=vim\n"
        );
        let updated = std::fs::read_to_string(&rc).unwrap();
        assert!(updated.starts_with("export EDITOR=vim\n"));
        assert!(updated.contains("# >>> fidim shell integration >>>"));
        assert!(updated.contains(&format!(
            "eval \"$(fidim --data-root {} env)\"",
            shlex_quote(&t.path().display().to_string())
        )));
        assert!(updated.contains(&shlex_quote(&shim_dir.display().to_string())));

        assert!(install_shell_integration(&rc, &[shim_dir], t.path())
            .unwrap()
            .is_none());
        assert_eq!(std::fs::read_to_string(&rc).unwrap(), updated);

        let next_root = t.path().join("moved");
        let next_backup =
            install_shell_integration(&rc, &[t.path().join("other bin")], &next_root).unwrap();
        let next_backup = next_backup.expect("updated rc file must be backed up");
        assert_eq!(std::fs::read_to_string(next_backup).unwrap(), updated);
        let updated_again = std::fs::read_to_string(&rc).unwrap();
        assert_eq!(updated_again.matches(SHELL_START_MARKER).count(), 1);
        assert_eq!(updated_again.matches(SHELL_END_MARKER).count(), 1);
        assert!(updated_again.contains(&next_root.display().to_string()));
        assert!(updated_again.contains("other bin"));
    }

    #[test]
    fn shell_integration_refuses_symlinked_rc_files() {
        use std::os::unix::fs::symlink;

        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("target");
        let rc = t.path().join(".bashrc");
        std::fs::write(&target, "user config\n").unwrap();
        symlink(&target, &rc).unwrap();

        let error = install_shell_integration(&rc, &[t.path().join("bin")], t.path()).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read_to_string(target).unwrap(), "user config\n");
    }
}
