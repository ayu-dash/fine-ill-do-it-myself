pub mod error;
pub mod manifest;
pub mod paths;
pub mod proc;
pub mod service;
pub mod shell;

pub use error::{Error, Result};
pub use manifest::{Lock, Manifest, PhpManager};
pub use paths::Paths;
pub use service::{Service, ServiceRegistry, ServiceState, ServiceStatus};

/// Human display name, used in the TUI header and release artefacts.
pub fn app_display_name() -> &'static str {
    "fine ill do it myself"
}
pub mod orchestrator;
pub mod php;
pub mod phpmyadmin;
pub mod supervisor;
pub mod web;
pub use orchestrator::{hosts_entries, list_projects};
pub use proc::{RealRunner, Runner};
pub use supervisor::{Env, PortPlan};
pub mod installer;

pub fn installer_defaults() -> &'static str {
    include_str!("default_manifest.toml")
}
pub mod database;
pub mod dns;
pub mod editor;
pub mod lang;
