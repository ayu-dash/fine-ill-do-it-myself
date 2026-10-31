//! Managed Composer: the pinned `composer.phar` downloaded by
//! `fidim install composer`, always run through the PHP that fidim manages so
//! it does not depend on whatever `php` happens to be on `PATH`.

use std::path::PathBuf;

use crate::error::Result;
use crate::installer::{self, Fetcher};
use crate::manifest::{Lock, Manifest};
use crate::supervisor::Env;

/// Manifest name of the composer artifact.
pub const NAME: &str = "composer";

/// Where the phar lives (`<data>/bin/composer`).
pub fn phar_path(env: &Env) -> PathBuf {
    env.paths.bin().join(NAME)
}

pub fn is_installed(env: &Env) -> bool {
    phar_path(env).is_file()
}

/// The PHP used to execute composer: the active version symlinked at
/// `<data>/bin/php/php`.
pub fn php_binary(env: &Env) -> Option<PathBuf> {
    let p = env.paths.bin().join("php").join("php");
    p.is_file().then_some(p)
}

/// Composer must not write to `$HOME`; keep its config and cache inside the
/// data root, like every other fidim artifact.
pub fn env_vars(env: &Env) -> Vec<(String, String)> {
    let home = env.paths.root().join("composer");
    vec![
        ("COMPOSER_HOME".to_string(), home.display().to_string()),
        (
            "COMPOSER_CACHE_DIR".to_string(),
            home.join("cache").display().to_string(),
        ),
    ]
}

/// Full argv for running composer with `args`.
pub fn cmd(env: &Env, args: &[String]) -> Result<Vec<String>> {
    let php = php_binary(env).ok_or_else(|| {
        crate::error::Error::Other(
            "php is not installed; install one with `fidim php install <version>`".into(),
        )
    })?;
    let mut cmd = vec![
        php.display().to_string(),
        phar_path(env).display().to_string(),
    ];
    cmd.extend(args.iter().cloned());
    Ok(cmd)
}

/// Install the pinned composer.phar unless it is already there.
pub fn ensure<F: Fetcher>(
    env: &Env,
    manifest: &Manifest,
    lock: &mut Lock,
    fetcher: &F,
) -> Result<PathBuf> {
    if is_installed(env) {
        return Ok(phar_path(env));
    }
    installer::install(env, manifest, lock, NAME, fetcher)
}

/// Version of the composer artifact this installation would install. Reads the
/// user manifest first (it may pin a different release) and falls back to the
/// bundled defaults.
pub fn pinned_version(env: &Env) -> Option<String> {
    Manifest::load(&env.manifest_path())
        .ok()
        .and_then(|m| m.find(NAME).map(|b| b.version.clone()))
        .or_else(|| {
            Manifest::parse(crate::installer_defaults())
                .ok()
                .and_then(|m| m.find(NAME).map(|b| b.version.clone()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::BinarySource;

    fn env(t: &tempfile::TempDir) -> Env {
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        env
    }

    fn touch_php(env: &Env) {
        let php = env.paths.bin().join("php").join("php");
        std::fs::create_dir_all(php.parent().unwrap()).unwrap();
        std::fs::write(&php, "").unwrap();
    }

    fn composer_manifest() -> Manifest {
        Manifest {
            binaries: vec![BinarySource {
                name: NAME.into(),
                version: "2.10.3".into(),
                url: "https://example.test/composer.phar".into(),
                sha256: String::new(),
                debs: Vec::new(),
                tar_gz: false,
                files: Vec::new(),
                install_dir: String::new(),
                strip_components: 0,
            }],
        }
    }

    #[derive(Default)]
    struct FakeFetcher {
        payload: Vec<u8>,
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Fetcher for FakeFetcher {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.payload.clone())
        }
    }

    #[test]
    fn cmd_puts_the_managed_php_before_the_phar() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        touch_php(&env);

        let cmd = cmd(&env, &["install".to_string()]).unwrap();

        assert_eq!(
            cmd,
            vec![
                env.paths.bin().join("php/php").display().to_string(),
                phar_path(&env).display().to_string(),
                "install".to_string()
            ]
        );
    }

    #[test]
    fn cmd_fails_without_a_managed_php() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);

        let err = cmd(&env, &[]).unwrap_err();

        assert!(
            format!("{err}").contains("php"),
            "error should mention php, got {err}"
        );
    }

    #[test]
    fn env_vars_keep_composer_home_under_the_data_root() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);

        let vars = env_vars(&env);

        let home = vars
            .iter()
            .find(|(k, _)| k == "COMPOSER_HOME")
            .map(|(_, v)| v.clone())
            .expect("COMPOSER_HOME must be set");
        let cache = vars
            .iter()
            .find(|(k, _)| k == "COMPOSER_CACHE_DIR")
            .map(|(_, v)| v.clone())
            .expect("COMPOSER_CACHE_DIR must be set");
        assert!(home.starts_with(&env.paths.root().display().to_string()));
        assert!(cache.starts_with(&env.paths.root().display().to_string()));
    }

    #[test]
    fn ensure_installs_the_pinned_phar() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let mut lock = Lock::default();
        let fetcher = FakeFetcher {
            payload: b"#!/usr/bin/env php\n".to_vec(),
            ..Default::default()
        };

        let path = ensure(&env, &composer_manifest(), &mut lock, &fetcher).unwrap();

        assert_eq!(path, phar_path(&env));
        assert_eq!(std::fs::read(&path).unwrap(), b"#!/usr/bin/env php\n");
        assert_eq!(fetcher.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(lock.binaries.contains_key(NAME));
    }

    #[test]
    fn ensure_is_a_noop_when_already_installed() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        std::fs::write(phar_path(&env), b"existing").unwrap();
        let mut lock = Lock::default();
        let fetcher = FakeFetcher {
            payload: b"new".to_vec(),
            ..Default::default()
        };

        let path = ensure(&env, &composer_manifest(), &mut lock, &fetcher).unwrap();

        assert_eq!(path, phar_path(&env));
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        assert_eq!(
            fetcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "must not download again"
        );
    }

    #[test]
    fn pinned_version_comes_from_the_bundled_manifest() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);

        assert_eq!(pinned_version(&env).as_deref(), Some("2.10.3"));
    }

    #[test]
    fn pinned_version_prefers_the_user_manifest() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let manifest = r#"[[binaries]]
name = "composer"
version = "9.9.9"
url = "https://example.test/composer.phar"
sha256 = ""
tar_gz = false
"#;
        std::fs::write(env.manifest_path(), manifest).unwrap();

        assert_eq!(pinned_version(&env).as_deref(), Some("9.9.9"));
    }

    #[test]
    fn is_installed_tracks_the_phar_file() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        assert!(!is_installed(&env));

        std::fs::write(phar_path(&env), b"x").unwrap();

        assert!(is_installed(&env));
    }
}
