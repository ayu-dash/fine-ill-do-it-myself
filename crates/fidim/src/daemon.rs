use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::Arc;

use fine_ill_do_it_myself::Env;

/// Pure dispatcher for one daemon request line.
pub fn handle(env: &Env, line: &str) -> String {
    let mut parts = line.split_whitespace();
    match parts.next() {
        Some("start") | Some("stop") => match run_cli(env, line.split_whitespace().next().unwrap())
        {
            Ok(o) => o,
            Err(e) => format!("error: {e}"),
        },
        Some("status") => {
            let mut projects = fine_ill_do_it_myself::list_projects(env);
            projects.sort();
            format!(
                "{}\nprojects: {}",
                status_snapshot(env),
                projects.join(", ")
            )
        }
        Some("ping") => "pong".into(),
        _ => "error: unknown command; supported: start, stop, status, ping".into(),
    }
}

fn status_snapshot(_env: &Env) -> String {
    "services: caddy, php".into()
}

fn run_cli(env: &Env, cmd: &str) -> Result<String, fine_ill_do_it_myself::Error> {
    let self_path = std::env::current_exe()?;
    let data_root = env.paths.root().display().to_string();
    let out = std::process::Command::new(self_path)
        .arg(format!("--data-root={data_root}"))
        .arg(cmd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()?;
    let mut text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    }
    Ok(text)
}

pub fn run_daemon(
    env: Env,
    socket_path: &std::path::Path,
) -> Result<(), fine_ill_do_it_myself::Error> {
    let _ = std::fs::remove_file(socket_path);
    match UnixListener::bind(socket_path) {
        Ok(l) => {
            let r = serve(env, l);
            let _ = std::fs::remove_file(socket_path);
            r
        }
        Err(e) => Err(fine_ill_do_it_myself::Error::Io(e)),
    }
}

pub fn serve(env: Env, listener: UnixListener) -> Result<(), fine_ill_do_it_myself::Error> {
    let env = Arc::new(env);
    loop {
        let (stream, _) = listener
            .accept()
            .map_err(fine_ill_do_it_myself::Error::Io)?;
        {
            let stream = stream;
            let mut reader = BufReader::new(stream.try_clone()?);
            let mut line = String::new();
            while reader.read_line(&mut line)? > 0 {
                let resp = handle(&env, line.trim());
                // follow-up reads must not consume half-written answers
                let _ = reader.get_mut().write_all(resp.as_bytes());
                let _ = reader.get_mut().write_all(b"\n");
                line.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_env() -> Env {
        let t = tempfile::tempdir().unwrap();
        Env::new(t.path().join("app"), t.path().join("Sites"))
    }

    #[test]
    fn ping_answers_pong() {
        assert_eq!(handle(&test_env(), "ping"), "pong");
    }

    #[test]
    fn unknown_command_errors() {
        assert!(handle(&test_env(), "boom").starts_with("error:"));
    }

    #[test]
    fn status_contains_projects_marker() {
        assert!(handle(&test_env(), "status").contains("projects:"));
    }

    #[test]
    fn handle_dispatches_all_commands() {
        for cmd in ["ping", "status", "nonsense"] {
            assert!(!handle(&test_env(), cmd).is_empty());
        }
    }

    #[test]
    fn run_daemon_creates_socket_file_on_tempdir() -> Result<(), fine_ill_do_it_myself::Error> {
        let t = tempfile::tempdir()?;
        let sock: PathBuf = t.path().join("d.sock");
        let env = test_env();
        let sock2 = sock.clone();
        let th = std::thread::spawn(move || run_daemon(env, &sock2));
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(sock.exists());
        let mut s = std::os::unix::net::UnixStream::connect(&sock)?;
        writeln!(s, "ping")?;
        let mut buf = [0u8; 5];
        use std::io::Read;
        s.read_exact(&mut buf)?;
        assert_eq!(&buf, b"pong\n");
        drop(th);
        Ok(())
    }
}
