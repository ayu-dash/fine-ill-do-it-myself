//! Shell integration: print `export` lines so managed clients (mysql, mariadb,
//! php, node, ...) can be used directly from the user's terminal.

use crate::supervisor::{shlex_quote, Env, PortPlan};

/// Render `export ...` lines for this data root. Meant to be evaluated:
/// `eval "$(fidim env)"`.
pub fn exports(env: &Env, ports: &PortPlan) -> String {
    let mut out = String::new();
    // managed clients live next to their runtime symlinks (bin/mariadb/mysql, ...)
    let client_dir = env.paths.bin().join("mariadb");
    if client_dir.is_dir() {
        out.push_str(&format!(
            "export PATH={}:\"$PATH\"\n",
            shlex_quote(&client_dir.display().to_string())
        ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with_root(root: &std::path::Path) -> Env {
        Env::new(root.join("app"), root.join("Sites"))
    }

    #[test]
    fn exports_point_path_and_mysql_clients_at_the_data_root() {
        let t = tempfile::tempdir().unwrap();
        let env = env_with_root(t.path());
        std::fs::create_dir_all(env.paths.bin().join("mariadb")).unwrap();
        let out = exports(&env, &PortPlan::default());
        let bin = env.paths.bin().join("mariadb").display().to_string();
        assert!(
            out.contains(&format!("export PATH='{bin}':\"$PATH\"")),
            "{out}"
        );
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
        let bin = env.paths.bin().join("mariadb").display().to_string();
        assert!(bin.contains(' '), "test needs a path with a space");
        assert!(
            out.contains(&format!("export PATH='{bin}':\"$PATH\"")),
            "{out}"
        );
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
}
