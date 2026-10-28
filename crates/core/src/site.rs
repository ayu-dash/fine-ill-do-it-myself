//! A project folder under `Sites` and the directory the web server actually
//! serves. Flat PHP projects serve their own folder; frameworks that keep the
//! entry point in `public/` (Laravel, Symfony) serve that subdirectory so the
//! application code above it is not reachable over HTTP.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Optional `<site>/.fidim.toml` written by the Node/Python templates.
#[derive(Debug, Default, Deserialize)]
struct SiteConfig {
    proxy_port: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub name: String,
    /// Absolute directory the vhost points at.
    pub docroot: PathBuf,
    /// When set, the vhost reverse-proxies to this local port instead of
    /// serving files (Node/Python applications).
    pub proxy: Option<u16>,
}

impl Site {
    /// Detect the docroot for `name` under `sites_root`. A `public/index.php`
    /// means the framework layout; anything else is served from the project
    /// folder itself. `.fidim.toml` can ask for a reverse proxy.
    pub fn detect(name: &str, sites_root: &Path) -> Self {
        let root = sites_root.join(name);
        let public = root.join("public");
        let docroot = if public.join("index.php").is_file() {
            public
        } else {
            root.clone()
        };
        let proxy = std::fs::read_to_string(root.join(".fidim.toml"))
            .ok()
            .and_then(|s| toml::from_str::<SiteConfig>(&s).ok())
            .and_then(|c| c.proxy_port);
        Self {
            name: name.to_string(),
            docroot,
            proxy,
        }
    }

    /// `docroot` as a string, ready to be embedded in a config file.
    pub fn docroot_str(&self) -> String {
        self.docroot.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_php_project_serves_its_own_folder() {
        let t = tempfile::tempdir().unwrap();
        let sites = t.path().join("Sites");
        std::fs::create_dir_all(sites.join("blog")).unwrap();
        std::fs::write(sites.join("blog/index.php"), "<?php").unwrap();

        let site = Site::detect("blog", &sites);

        assert_eq!(site.name, "blog");
        assert_eq!(site.docroot, sites.join("blog"));
    }

    #[test]
    fn public_index_php_moves_the_docroot_into_public() {
        let t = tempfile::tempdir().unwrap();
        let sites = t.path().join("Sites");
        std::fs::create_dir_all(sites.join("shop/public")).unwrap();
        std::fs::write(sites.join("shop/public/index.php"), "<?php").unwrap();
        std::fs::write(sites.join("shop/.env"), "APP_KEY=x").unwrap();

        let site = Site::detect("shop", &sites);

        assert_eq!(site.docroot, sites.join("shop/public"));
    }

    #[test]
    fn a_public_folder_without_an_entry_point_stays_flat() {
        let t = tempfile::tempdir().unwrap();
        let sites = t.path().join("Sites");
        std::fs::create_dir_all(sites.join("docs/public")).unwrap();

        let site = Site::detect("docs", &sites);

        assert_eq!(site.docroot, sites.join("docs"));
    }
}
