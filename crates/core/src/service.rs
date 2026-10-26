use std::path::PathBuf;

use crate::error::Result;
use serde::{Deserialize, Serialize};

/// A service identity exposed to the UI/CLI layer. Concrete services are the
/// caller's own structs; this is a thin, testable view of what's running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceState {
    Running,
    Stopped,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub name: String,
    pub state: ServiceState,
    pub pid: Option<u32>,
}

pub trait Service {
    fn name(&self) -> &str;
    /// Build the exact command (argv) that should be launched in background.
    fn start_command(&self) -> Vec<String>;
    /// Graceful stop: send SIGTERM (or SQL SHUTDOWN for mariadb) to current pid.
    fn stop_pid(&mut self) -> Result<Option<u32>>;
    fn status(&self) -> ServiceStatus;
    fn logs_path(&self) -> PathBuf;
    fn healthcheck(&self) -> bool;
}

#[derive(Debug, Clone, Default)]
pub struct ServiceRegistry {
    pub names: Vec<String>,
}

impl ServiceRegistry {
    pub fn register(&mut self, name: impl Into<String>) {
        self.names.push(name.into());
    }
}

/// Very small in-memory action/message plumbing shared by CLI and TUI layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start(String),
    Stop(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeService {
        pid: Option<u32>,
        healthy: bool,
    }

    impl Service for FakeService {
        fn name(&self) -> &str {
            "fake"
        }
        fn start_command(&self) -> Vec<String> {
            vec!["echo".into(), "fake".into()]
        }
        fn stop_pid(&mut self) -> Result<Option<u32>> {
            let p = self.pid;
            self.pid = None;
            Ok(p)
        }
        fn status(&self) -> ServiceStatus {
            ServiceStatus {
                name: "fake".into(),
                state: if self.pid.is_some() {
                    ServiceState::Running
                } else {
                    ServiceState::Stopped
                },
                pid: self.pid,
            }
        }
        fn logs_path(&self) -> PathBuf {
            PathBuf::from("/tmp/fake.log")
        }
        fn healthcheck(&self) -> bool {
            self.healthy
        }
    }

    #[test]
    fn service_reports_running_when_pid_present() {
        let svc = FakeService {
            pid: Some(1),
            healthy: true,
        };
        assert_eq!(svc.status().state, ServiceState::Running);
    }

    #[test]
    fn service_stop_clears_pid() {
        let mut svc = FakeService {
            pid: Some(7),
            healthy: true,
        };
        assert_eq!(svc.stop_pid().unwrap(), Some(7));
        assert_eq!(svc.status().state, ServiceState::Stopped);
    }

    #[test]
    fn service_healthcheck_flag_used() {
        let svc = FakeService {
            pid: Some(1),
            healthy: false,
        };
        assert!(!svc.healthcheck());
    }

    #[test]
    fn registry_collects_names() {
        let mut r = ServiceRegistry::default();
        r.register("caddy");
        r.register("php-fpm");
        assert_eq!(r.names, vec!["caddy".to_string(), "php-fpm".to_string()]);
    }
}
