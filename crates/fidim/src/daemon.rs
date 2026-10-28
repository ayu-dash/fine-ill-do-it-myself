use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use fine_ill_do_it_myself::Env;

/// Set from the SIGTERM/SIGINT handler so the accept loop can clean up.
static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn request_stop(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            request_stop as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            request_stop as *const () as libc::sighandler_t,
        );
    }
}

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

fn status_snapshot(env: &Env) -> String {
    let ports =
        fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
    let mut lines = vec!["services:".to_string()];
    for status in fine_ill_do_it_myself::orchestrator::status_all(
        env,
        &ports,
        &fine_ill_do_it_myself::proc::RealRunner,
    ) {
        lines.push(format!("  {}: {:?}", status.name, status.state));
    }
    lines.join("\n")
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
    STOP.store(false, Ordering::SeqCst);
    install_signal_handlers();
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
    serve_with(env, listener, || STOP.load(Ordering::SeqCst))
}

/// Serve until `stop()` asks to exit; `run_daemon` passes the signal flag.
/// The listener is non-blocking so an idle daemon notices the request and can
/// remove its socket before the process ends.
fn serve_with(
    env: Env,
    listener: UnixListener,
    stop: impl Fn() -> bool,
) -> Result<(), fine_ill_do_it_myself::Error> {
    listener.set_nonblocking(true)?;
    let env = Arc::new(env);
    loop {
        if stop() {
            return Ok(());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok();
                stream
                    .set_read_timeout(Some(Duration::from_millis(250)))
                    .ok();
                let mut reader = BufReader::new(stream.try_clone()?);
                let mut line = String::new();
                loop {
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            let resp = handle(&env, line.trim());
                            // follow-up reads must not consume half-written answers
                            let _ = reader.get_mut().write_all(resp.as_bytes());
                            let _ = reader.get_mut().write_all(b"\n");
                            line.clear();
                        }
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            if stop() {
                                return Ok(());
                            }
                        }
                        Err(e) => return Err(fine_ill_do_it_myself::Error::Io(e)),
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(fine_ill_do_it_myself::Error::Io(e)),
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
    fn status_lists_the_managed_services() {
        let out = handle(&test_env(), "status");

        assert!(out.contains("caddy"), "{out}");
        assert!(out.contains("mariadb"), "{out}");
        assert!(out.contains("projects:"), "{out}");
    }

    #[test]
    fn serve_stops_when_asked_to_shut_down() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;

        let t = tempfile::tempdir().unwrap();
        let sock = t.path().join("d.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let served = serve_with(test_env(), listener, move || flag.load(Ordering::SeqCst));
            let _ = tx.send(served.is_ok());
        });
        std::thread::sleep(Duration::from_millis(100));

        let mut s = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        writeln!(s, "ping").unwrap();
        let mut buf = [0u8; 5];
        use std::io::Read;
        s.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"pong\n");
        drop(s);

        stop.store(true, Ordering::SeqCst);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).ok(),
            Some(true),
            "the daemon must stop when asked instead of leaking its socket"
        );
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
