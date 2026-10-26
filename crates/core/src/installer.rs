//! Install binaries from the manifest: fetch, verify checksum, extract,
//! chmod, record into lock.toml. The `Fetcher` trait keeps this testable
//! offline (HTTP impl uses reqwest blocking).
use std::path::Path;
use std::{fs, path::PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::manifest::{Lock, Manifest};
use crate::supervisor::Env;

pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>>;
}

#[derive(Debug, Clone, Default)]
pub struct HttpFetcher;

impl Fetcher for HttpFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        let bytes = reqwest::blocking::Client::new()
            .get(url)
            .send()?
            .error_for_status()?
            .bytes()?;
        Ok(bytes.to_vec())
    }
}

/// Downloaded tarball contents may contain either a single binary or several
/// files (caddy ships just the binary plus licenses). When `files` is given
/// explicitly, copy those names verbatim into `dest_dir`.
pub fn unpack_binary(
    bytes: &[u8],
    tar_gz: bool,
    files: &[String],
    staging: &Path,
    dest_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let mut moved = Vec::new();
    if tar_gz {
        let gz = flate2::read::GzDecoder::new(bytes);
        let mut archive = tar::Archive::new(gz);
        fs::create_dir_all(staging)?;
        archive.unpack(staging)?;
        if !files.is_empty() {
            for f in files {
                let src = staging.join(f);
                if !src.exists() {
                    return Err(Error::Other(format!("missing {f} inside tarball")));
                }
                let out = dest_dir.join(f.trim_start_matches("./"));
                fs::create_dir_all(out.parent().unwrap_or(dest_dir))?;
                fs::rename(&src, &out)?;
                moved.push(out);
            }
        } else {
            // default: move the single regular file at the archive root
            let mut found = Vec::new();
            for e in fs::read_dir(staging)?.flatten() {
                if e.path().is_file() {
                    found.push(e.path());
                }
            }
            if found.len() != 1 {
                return Err(Error::Other(format!(
                    "tarball has {} top-level files; specify `files` in the manifest",
                    found.len()
                )));
            }
            let out = dest_dir.join(found[0].file_name().unwrap());
            fs::rename(&found[0], &out)?;
            moved.push(out);
        }
    } else {
        fs::create_dir_all(dest_dir)?;
        if let Some(_parent) = staging.parent() {
            let out = dest_dir;
            fs::write(out, bytes)?;
            moved.push(out.to_path_buf());
        }
    }
    for p in &moved {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(p)?.permissions();
            perm.set_mode(0o755);
            fs::set_permissions(p, perm)?;
        }
    }
    let _ = fs::remove_dir_all(staging);
    Ok(moved)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Install (or update) a binary by manifest name, e.g. `caddy`, `php-fpm-8.4`.
/// Destination file is `bin/<name>` for php entries, or `files` names for tarballs.
pub fn install<F: Fetcher>(
    env: &Env,
    manifest: &Manifest,
    lock: &mut Lock,
    name: &str,
    fetcher: &F,
) -> Result<PathBuf> {
    let src = manifest
        .find(name)
        .ok_or_else(|| Error::MissingBinary(name.to_string()))?
        .clone();
    let staging = env.paths.bin().join(format!(".{name}-staging"));
    let bytes = fetcher.fetch(&src.url)?;
    let hex = sha256_hex(&bytes);
    // verify pinned/recorded checksum
    let expected: Option<&String> = {
        let pinned = (!src.sha256.is_empty()).then_some(&src.sha256);
        pinned.or_else(|| {
            lock.binaries
                .get(name)
                .filter(|h| !h.sha256.is_empty())
                .map(|e| &e.sha256)
        })
    };
    if let Some(expected) = expected {
        if &hex != expected {
            return Err(Error::ChecksumMismatch(name.to_string()));
        }
    }
    let dest = env.paths.bin();
    let targets = if src.tar_gz {
        unpack_binary(&bytes, true, &src.files, &staging, &dest)?
    } else {
        unpack_binary(&bytes, false, &src.files, &staging, &dest)?
    };
    // record sha so future reinstalls/re-images verify integrity
    let entry = lock.binaries.entry(name.to_string()).or_default();
    entry.sha256 = hex.clone();
    let _ = entry;
    lock.save(&env.lock_path())?;
    // php entries: expose a short alias php-fpm-<version>
    if let Some(v) = name.strip_prefix("php-cli-") {
        let alias = dest.join(format!("php-{v}"));
        let _ = std::os::unix::fs::symlink(dest.join(name), &alias);
    }
    Ok(targets.first().cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeFetcher(Vec<u8>);

    impl Fetcher for FakeFetcher {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>> {
            Ok(self.0.clone())
        }
    }

    fn tar_gz_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut tar = tar::Builder::new(&mut w);
            for (name, data) in entries {
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_mode(0o755);
                tar.append_data(&mut h, name, *data).unwrap();
            }
            tar.finish().unwrap();
        }
        w.finish().unwrap()
    }

    fn json_manifest(name: &str) -> Manifest {
        let m = format!(
            r#"[[binaries]]
name = "{name}"
version = "1"
url = "http://x/{name}.tar.gz"
sha256 = ""
tar_gz = true
files = []
"#
        );
        toml::from_str(&m).unwrap()
    }

    #[test]
    fn unpack_single_file_tarball_into_bin() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        let manifest = json_manifest("caddy");
        let mut lock = Lock::default();
        let f = FakeFetcher(tar_gz_bytes(&[(
            "caddy",
            b"#!/bin/sh
"
            .as_slice(),
        )]));
        let out = install(&env, &manifest, &mut lock, "caddy", &f).unwrap();
        assert_eq!(out, env.paths.bin().join("caddy"));
        assert!(env.caddy_bin().exists());
    }

    #[test]
    fn installer_verifies_recorded_checksum_on_update() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        let manifest = json_manifest("caddy");
        let bytes = b"payload";
        let gz = tar_gz_bytes(&[("caddy", bytes)]);
        let want = sha256_hex(&gz);
        let mut lock = Lock::default();
        let e = lock.binaries.entry("caddy".into()).or_default();
        e.sha256 = want.clone();
        let f = FakeFetcher(gz.clone());
        install(&env, &manifest, &mut lock, "caddy", &f).unwrap();
        // now corrupt expectation -> must fail
        let mut bad_manifest = json_manifest("caddy");
        bad_manifest.binaries[0].sha256 = "deadbeef".into();
        let err = install(&env, &bad_manifest, &mut lock, "caddy", &FakeFetcher(gz)).unwrap_err();
        assert!(matches!(err, Error::ChecksumMismatch(_)));
    }

    #[test]
    fn installer_requires_manifest_entry() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        let mut lock = Lock::default();
        let err = install(
            &env,
            &Manifest::default(),
            &mut lock,
            "x",
            &FakeFetcher(vec![]),
        )
        .unwrap_err();
        assert!(matches!(err, Error::MissingBinary(_)));
    }
}
