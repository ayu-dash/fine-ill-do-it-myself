//! Install binaries from the manifest: fetch, verify checksum, extract,
//! chmod, record into lock.toml. The `Fetcher` trait keeps this testable
//! offline (default impl uses system curl).
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use std::{fs, io, path::PathBuf};

use sha2::{Digest, Sha256};

use crate::cancel::CancelToken;
use crate::error::{Error, Result};
use crate::manifest::{Lock, Manifest};
use crate::supervisor::Env;

pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>>;
}

/// Download progress callback: `(done, total)`. `total == 0` means the server
/// did not report a size, so the UI shows an indeterminate bar.
pub type ProgressSink = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// `curl`-backed fetcher. Plain construction keeps the original blocking
/// behaviour; [`CurlFetcher::with_progress`] adds a progress meter and lets a
/// [`CancelToken`] abort the transfer (used by the TUI worker so a slow
/// download can never freeze or trap the interface).
#[derive(Clone, Default)]
pub struct CurlFetcher {
    cancel: Option<CancelToken>,
    progress: Option<ProgressSink>,
}

impl std::fmt::Debug for CurlFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurlFetcher")
            .field("cancellable", &self.cancel.is_some())
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

impl CurlFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fetcher whose transfer reports percentages and stops when `cancel` is
    /// set (the curl child is killed within ~40 ms).
    pub fn with_progress(
        cancel: CancelToken,
        sink: impl Fn(u64, u64) + Send + Sync + 'static,
    ) -> Self {
        Self {
            cancel: Some(cancel),
            progress: Some(Arc::new(sink)),
        }
    }

    fn fetch_blocking(&self, url: &str) -> Result<Vec<u8>> {
        let out = Command::new("curl")
            .args(["-fsSL", "--", url])
            .output()
            .map_err(|e| Error::Other(format!("curl exec failed: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(Error::Other(format!("curl failed: {stderr}")));
        }
        Ok(out.stdout)
    }

    fn fetch_streaming(&self, url: &str) -> Result<Vec<u8>> {
        let mut child = Command::new("curl")
            .args(["-fsSL", "--progress-bar", "--", url])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Other(format!("curl exec failed: {e}")))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Other("curl stdout unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Other("curl stderr unavailable".into()))?;

        // Body and progress meter are two pipes: drain them concurrently so
        // neither side can fill its buffer and stall curl.
        let body = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            std::io::copy(&mut stdout, &mut bytes).map(|_| bytes)
        });
        let sink = self.progress.clone();
        let meter = std::thread::spawn(move || {
            let mut err = stderr;
            let mut buf = [0u8; 512];
            let mut carry = String::new();
            let mut last = None;
            loop {
                match err.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        carry.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if let Some(pct) = parse_progress_percent(&carry) {
                            if last != Some(pct) {
                                last = Some(pct);
                                if let Some(sink) = &sink {
                                    sink(pct, 100);
                                }
                            }
                        }
                        if carry.len() > 512 {
                            let keep = carry.chars().count().saturating_sub(64);
                            carry = carry.chars().skip(keep).collect();
                        }
                    }
                }
            }
            carry
        });

        let status = loop {
            if self.cancel.as_ref().is_some_and(CancelToken::is_cancelled) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = body.join();
                let _ = meter.join();
                return Err(Error::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                Err(e) => {
                    let _ = child.kill();
                    let _ = body.join();
                    let _ = meter.join();
                    return Err(Error::Other(format!("curl wait failed: {e}")));
                }
            }
        };
        let bytes = body
            .join()
            .map_err(|_| Error::Other("curl reader panicked".into()))?
            .map_err(Error::Io)?;
        let tail = meter.join().unwrap_or_default();
        if !status.success() {
            let detail = tail
                .rsplit(['\r', '\n'])
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .trim()
                .to_string();
            return Err(Error::Other(format!("curl failed: {detail}")));
        }
        Ok(bytes)
    }
}

impl Fetcher for CurlFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        if self.cancel.is_none() && self.progress.is_none() {
            return self.fetch_blocking(url);
        }
        self.fetch_streaming(url)
    }
}

/// Last percentage printed by curl's progress bar in `text`.
///
/// Output looks like `####  45.0%\r####  90.0%\r`, so the number is read
/// backwards from the final `%`. Pure function: unit-tested offline.
pub(crate) fn parse_progress_percent(text: &str) -> Option<u64> {
    let pct = text.rfind('%')?;
    let before = &text[..pct];
    let start = before
        .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map(|i| i + 1)
        .unwrap_or(0);
    let value: f64 = before[start..].trim_end_matches('.').parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some(value.clamp(0.0, 100.0).round() as u64)
}

pub type HttpFetcher = CurlFetcher;

/// Downloaded tarball contents may contain either a single binary or several
/// files (caddy ships just the binary plus licenses). When `files` is given
/// explicitly, copy those names verbatim into `dest_dir`.
fn find_file_in_dir(root: &Path, rel: &str) -> Option<PathBuf> {
    let direct = root.join(rel);
    if direct.exists() {
        return Some(direct);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.is_file()
                    && (p.ends_with(rel) || p.file_name().map(|n| n == rel).unwrap_or(false))
                {
                    return Some(p);
                }
            }
        }
    }
    None
}

pub fn unpack_binary(
    bytes: &[u8],
    files: &[String],
    staging: &Path,
    dest_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let mut moved = Vec::new();
    {
        let gz = flate2::read::GzDecoder::new(bytes);
        let mut archive = tar::Archive::new(gz);
        fs::create_dir_all(staging)?;
        archive.unpack(staging)?;
        if !files.is_empty() {
            for f in files {
                let (src_rel, dest_rel) = if let Some((s, d)) = f.split_once(':') {
                    (s.trim(), d.trim())
                } else {
                    (f.as_str(), f.trim_start_matches("./"))
                };
                let src = find_file_in_dir(staging, src_rel)
                    .ok_or_else(|| Error::Other(format!("missing {src_rel} inside tarball")))?;
                let out = dest_dir.join(dest_rel);
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
    }
    for p in &moved {
        make_executable(p)?;
    }
    let _ = fs::remove_dir_all(staging);
    Ok(moved)
}

/// Single-file artifact (composer.phar): the fetched bytes are the binary.
fn write_single_file(bytes: &[u8], dest: &Path) -> Result<Vec<PathBuf>> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(dest, bytes)?;
    make_executable(dest)?;
    Ok(vec![dest.to_path_buf()])
}

fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(path)?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(path, perm)?;
    }
    Ok(())
}

/// Extract a tarball into `dest_dir`, stripping `strip` leading path components.
/// Used by `install_to_dir` for full-directory runtimes (Node, Go, Python, Java).
pub fn unpack_dir(bytes: &[u8], dest_dir: &Path, strip: u32) -> Result<()> {
    use io::Read;
    let gz = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(gz);
    fs::create_dir_all(dest_dir)?;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        // Strip the requested number of leading components
        let stripped: PathBuf = path.components().skip(strip as usize).collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        let out = dest_dir.join(&stripped);
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                fs::create_dir_all(&out)?;
            }
            tar::EntryType::Symlink =>
            {
                #[cfg(unix)]
                if let Ok(Some(target)) = entry.header().link_name() {
                    if let Some(parent) = out.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let _ = fs::remove_file(&out);
                    let _ = std::os::unix::fs::symlink(&target, &out);
                }
            }
            _ => {
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut data = Vec::new();
                entry.read_to_end(&mut data)?;
                fs::write(&out, &data)?;
                #[cfg(unix)]
                if let Ok(mode) = entry.header().mode() {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&out, fs::Permissions::from_mode(mode))?;
                }
            }
        }
    }
    Ok(())
}

/// Install a full-directory runtime: extract into `runtimes/<install_dir>/` then
/// create symlinks in `bin/<lang>/<lang>-<version>` for each listed binary.
/// Returns the path to the primary symlink (first entry in `files`).
pub fn install_to_dir<F: Fetcher>(
    env: &Env,
    src: &crate::manifest::BinarySource,
    bytes: &[u8],
) -> Result<PathBuf> {
    let runtime_root = env.paths.runtimes().join(&src.install_dir);
    // If already extracted, remove old installation
    if runtime_root.exists() {
        fs::remove_dir_all(&runtime_root)?;
    }
    unpack_dir(bytes, &runtime_root, src.strip_components)?;

    // Create symlinks in bin/<lang>/
    let bin = env.paths.bin();
    let mut first_link: Option<PathBuf> = None;
    for f in &src.files {
        let (src_rel, dest_rel) = if let Some((s, d)) = f.split_once(':') {
            (s.trim(), d.trim())
        } else {
            (f.as_str(), f.trim_start_matches("./"))
        };
        let runtime_bin = runtime_root.join(src_rel);
        if !runtime_bin.exists() {
            // Try searching for it
            if let Some(found) = find_file_in_dir(&runtime_root, src_rel) {
                let link_dest = bin.join(dest_rel);
                if let Some(parent) = link_dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                #[cfg(unix)]
                {
                    let _ = fs::remove_file(&link_dest);
                    std::os::unix::fs::symlink(&found, &link_dest)?;
                }
                #[cfg(not(unix))]
                fs::copy(&found, &link_dest).map(|_| ())?;
                if first_link.is_none() {
                    first_link = Some(link_dest);
                }
            }
        } else {
            let link_dest = bin.join(dest_rel);
            if let Some(parent) = link_dest.parent() {
                fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            {
                let _ = fs::remove_file(&link_dest);
                std::os::unix::fs::symlink(&runtime_bin, &link_dest)?;
            }
            #[cfg(not(unix))]
            fs::copy(&runtime_bin, &link_dest).map(|_| ())?;
            if first_link.is_none() {
                first_link = Some(link_dest);
            }
        }
    }
    Ok(first_link.unwrap_or_else(|| runtime_root.clone()))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Install (or update) a binary by manifest name, e.g. `caddy`, `php-fpm-8.4`, `node-22`.
/// When `install_dir` is set in the manifest, the full tarball is extracted into
/// `runtimes/<install_dir>/` and binary symlinks are created in `bin/<lang>/`.
/// Otherwise, individual `files` entries are extracted directly into `bin/`.
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
    // Recipes made of pinned packages (Apache) build a runtime tree instead of
    // extracting the single `url` artifact.
    if !src.debs.is_empty() {
        return crate::web::apache_bundle::install(env, &src, lock, fetcher);
    }
    let staging = env.paths.bin().join(format!(".{name}-staging"));
    let bytes = fetcher.fetch(&src.url)?;
    let hex = sha256_hex(&bytes);
    // verify pinned/recorded checksum
    let expected: Option<&String> = {
        let pinned = (!src.sha256.is_empty()).then_some(&src.sha256);
        pinned.or_else(|| {
            lock.binaries
                .get(name)
                .filter(|h| !h.sha256.is_empty() && h.url == src.url)
                .map(|e| &e.sha256)
        })
    };
    if let Some(expected) = expected {
        if &hex != expected {
            return Err(Error::ChecksumMismatch(name.to_string()));
        }
    }
    // record sha so future reinstalls/re-images verify integrity
    let entry = lock.binaries.entry(name.to_string()).or_default();
    entry.url = src.url.clone();
    entry.sha256 = hex.clone();
    let _ = entry;
    lock.save(&env.lock_path())?;

    // Full-directory install (Node, Go, Python, Java)
    if !src.install_dir.is_empty() {
        let primary = install_to_dir::<F>(env, &src, &bytes)?;
        return Ok(primary);
    }

    // Single-binary / selected-files install (Caddy, PHP-FPM, ...)
    let dest = env.paths.bin();
    let targets = if src.tar_gz {
        unpack_binary(&bytes, &src.files, &staging, &dest)?
    } else {
        write_single_file(&bytes, &dest.join(name))?
    };
    // expose the binary under its manifest name (e.g. php-fpm-8.4)
    if let Some(first) = targets.first() {
        let want = dest.join(name);
        if *first != want {
            let _ = fs::remove_file(&want);
            fs::hard_link(first, &want).or_else(|_| fs::copy(first, &want).map(|_| ()))?;
        }
    }
    Ok(dest.join(name))
}

#[cfg(test)]
pub mod tests_support {
    /// Build a small .tar.gz in memory for extraction tests.
    pub fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut tar = tar::Builder::new(&mut w);
            for (name, data) in entries {
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_mode(0o644);
                tar.append_data(&mut h, name, *data).unwrap();
            }
            tar.finish().unwrap();
        }
        w.finish().unwrap()
    }
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

    /// Single-file artifacts (composer.phar) are not tarballs: the bytes are
    /// the binary itself, so they must land in `bin/<name>` ready to execute.
    #[test]
    fn unpack_single_file_artifact_into_bin() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        let manifest: Manifest = toml::from_str(
            r#"[[binaries]]
name = "composer"
version = "2.10.3"
url = "https://example.test/composer.phar"
sha256 = ""
tar_gz = false
"#,
        )
        .unwrap();
        let payload = b"#!/usr/bin/env php\n<?php echo 'composer';\n";
        let f = FakeFetcher(payload.to_vec());
        let mut lock = Lock::default();

        let out = install(&env, &manifest, &mut lock, "composer", &f).unwrap();

        assert_eq!(out, env.paths.bin().join("composer"));
        assert_eq!(std::fs::read(&out).unwrap(), payload);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&out).unwrap().permissions().mode();
            assert!(
                mode & 0o111 != 0,
                "composer must be executable, mode {mode:o}"
            );
        }
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

    /// Node/Go/Python tarballs wrap everything under a versioned top-level dir,
    /// e.g. `node-v20.18.0-linux-x64/bin/node`. Ensure `files = ["bin/node:node/node-20"]`
    /// correctly locates and moves the binary into `bin/node/node-20`.
    #[test]
    fn unpack_nested_tarball_with_files_mapping() {
        let t = tempfile::tempdir().unwrap();
        let staging = t.path().join("staging");
        let dest = t.path().join("bin");
        std::fs::create_dir_all(&dest).unwrap();

        // Simulate node-v20.18.0-linux-x64/bin/node inside tarball
        let bytes = tar_gz_bytes(&[
            ("node-v20.18.0-linux-x64/bin/node", b"#!/bin/node\n"),
            ("node-v20.18.0-linux-x64/bin/npm", b"#!/bin/npm\n"),
            ("node-v20.18.0-linux-x64/bin/npx", b"#!/bin/npx\n"),
        ]);

        let files: Vec<String> = vec![
            "bin/node:node/node-20".to_string(),
            "bin/npm:node/npm-20".to_string(),
            "bin/npx:node/npx-20".to_string(),
        ];

        let moved = unpack_binary(&bytes, &files, &staging, &dest).unwrap();
        assert_eq!(moved.len(), 3);
        assert!(
            dest.join("node/node-20").exists(),
            "node-20 binary should exist"
        );
        assert!(dest.join("node/npm-20").exists(), "npm-20 should exist");
        assert!(dest.join("node/npx-20").exists(), "npx-20 should exist");
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

    /// Verify that install_dir + strip_components correctly extracts the full
    /// Node tarball tree into runtimes/node/20/ and symlinks bin/ entries.
    #[test]
    fn install_dir_extracts_full_tree_and_symlinks_bins() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().join("app"), t.path().join("sites"));
        env.ensure_dirs().unwrap();

        // Simulate node-v20.18.0-linux-x64/ tarball structure
        let bytes = tar_gz_bytes(&[
            ("node-v20.18.0-linux-x64/bin/node", b"#!/bin/node\n"),
            (
                "node-v20.18.0-linux-x64/bin/npm",
                b"#!/usr/bin/env node\nnpm\n",
            ),
            (
                "node-v20.18.0-linux-x64/lib/node_modules/npm/package.json",
                b"{}",
            ),
        ]);

        let manifest_toml = r#"
[[binaries]]
name = "node-20"
version = "20.18.0"
url = "http://x/node-20.tar.gz"
sha256 = ""
tar_gz = true
install_dir = "node/20"
strip_components = 1
files = ["bin/node:node/node-20", "bin/npm:node/npm-20"]
"#;
        let manifest: Manifest = toml::from_str(manifest_toml).unwrap();
        let mut lock = Lock::default();
        let f = FakeFetcher(bytes);

        install(&env, &manifest, &mut lock, "node-20", &f).unwrap();

        // Runtime dir should have full tree extracted
        let rt = env.paths.runtimes().join("node/20");
        assert!(
            rt.join("bin/node").exists(),
            "runtime bin/node should exist"
        );
        assert!(
            rt.join("lib/node_modules/npm/package.json").exists(),
            "npm modules extracted"
        );

        // Symlinks should exist in bin/node/
        let bin_node = env.paths.bin().join("node/node-20");
        let bin_npm = env.paths.bin().join("node/npm-20");
        assert!(bin_node.exists(), "bin/node/node-20 symlink should exist");
        assert!(bin_npm.exists(), "bin/node/npm-20 symlink should exist");
    }

    #[test]
    fn curl_fetcher_fetches_file_url() {
        let t = tempfile::tempdir().unwrap();
        let sample = t.path().join("sample.txt");
        fs::write(&sample, b"hello from curl").unwrap();
        let url = format!("file://{}", sample.to_str().unwrap());
        let fetcher = CurlFetcher::default();
        let data = fetcher.fetch(&url).unwrap();
        assert_eq!(data, b"hello from curl");
    }

    #[test]
    fn curl_fetcher_fails_on_nonexistent() {
        let fetcher = CurlFetcher::default();
        let res = fetcher.fetch("file:///nonexistent/path/that/does/not/exist");
        assert!(res.is_err());
    }

    #[test]
    fn progress_parser_reads_the_last_percentage() {
        assert_eq!(parse_progress_percent("###  45.0%"), Some(45));
        assert_eq!(parse_progress_percent("\r####  12.5%\r"), Some(13));
        assert_eq!(parse_progress_percent("100.0%"), Some(100));
        assert_eq!(parse_progress_percent(" 0.0%"), Some(0));
        assert_eq!(parse_progress_percent("no meter here"), None);
        assert_eq!(parse_progress_percent("%"), None);
    }

    #[test]
    fn progress_parser_clamps_out_of_range_values() {
        assert_eq!(parse_progress_percent("1234.5%"), Some(100));
    }

    #[test]
    fn streaming_fetch_still_returns_the_body() {
        let t = tempfile::tempdir().unwrap();
        let sample = t.path().join("sample.bin");
        fs::write(&sample, vec![7u8; 64 * 1024]).unwrap();
        let url = format!("file://{}", sample.to_str().unwrap());
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(u64, u64)>::new()));
        let sink = seen.clone();

        let fetcher = CurlFetcher::with_progress(CancelToken::new(), move |done, total| {
            sink.lock().unwrap().push((done, total));
        });
        let data = fetcher.fetch(&url).unwrap();

        assert_eq!(data.len(), 64 * 1024, "the whole body must arrive");
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter().all(|(_, total)| *total == 100),
            "progress is reported as a percentage: {seen:?}"
        );
    }

    #[test]
    fn a_cancelled_token_aborts_the_transfer() {
        let t = tempfile::tempdir().unwrap();
        let sample = t.path().join("sample.bin");
        fs::write(&sample, vec![1u8; 1024]).unwrap();
        let url = format!("file://{}", sample.to_str().unwrap());
        let cancel = CancelToken::new();
        cancel.cancel();

        let fetcher = CurlFetcher::with_progress(cancel, |_, _| {});
        let err = fetcher.fetch(&url).unwrap_err();

        assert!(
            matches!(err, Error::Cancelled),
            "a cancelled download must not pretend it succeeded: {err}"
        );
    }
}
