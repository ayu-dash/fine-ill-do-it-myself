pub mod error;
pub mod manifest;
pub mod paths;
pub mod proc;
pub mod service;

pub use error::{Error, Result};
pub use manifest::{Manifest, PhpManager};
pub use paths::Paths;
pub use service::{Service, ServiceRegistry, ServiceState, ServiceStatus};

/// Human display name, used in the TUI header and release artefacts.
pub fn app_display_name() -> &'static str {
    "fine ill do it myself"
}
pub mod orchestrator;
pub mod php;
pub mod supervisor;
pub mod web;
pub use proc::{RealRunner, Runner};
pub use supervisor::{Env, PortPlan};
pub mod installer;

pub fn installer_defaults() -> &'static str {
    include_str!("default_manifest.toml")
}
