use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where to fetch every managed binary from. A single manifest file so
/// versions/checksums are easy to update without touching code.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    #[serde(default)]
    pub binaries: Vec<BinarySource>,
}

/// One downloadable artifact. `tar_gz = true` means the URL is a tarball whose
/// only file is the binary (or tools after extraction into `dest`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinarySource {
    pub name: String,
    pub version: String,
    pub url: String,
    /// Optional fixed checksum; empty = accept and record on first download
    /// (recorded download instance is validated against it thereafter).
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub tar_gz: bool,
    /// File names inside the tarball to preserve relative to its root.
    /// Format: `"src_rel:dest_rel"` or `"name"` (src = dest).
    #[serde(default)]
    pub files: Vec<String>,
    /// When set, extract the entire tarball into `.fidim/runtimes/<install_dir>/`
    /// instead of copying individual files. `files` then lists which binaries
    /// to symlink from the runtime dir into `bin/<lang>/`.
    /// Example: `install_dir = "node/22"` → extracts to `runtimes/node/22/`.
    #[serde(default)]
    pub install_dir: String,
    /// Number of leading path components to strip when extracting with `install_dir`.
    /// Node/Java tarballs have a wrapper dir (e.g. `node-v22-linux-x64/`), so use 1.
    /// Go tarballs have a `go/` prefix, so also use 1.
    #[serde(default)]
    pub strip_components: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestEntry {
    pub url: String,
    pub sha256: String,
    pub installed_at: String,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        let s = std::fs::read_to_string(path)?;
        toml::from_str(&s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let s = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, s)
    }
    pub fn find(&self, name: &str) -> Option<&BinarySource> {
        self.binaries.iter().find(|b| b.name == name)
    }
}

/// Record of installed binaries so subsequent launches can verify integrity.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Lock {
    #[serde(default)]
    pub binaries: std::collections::BTreeMap<String, ManifestEntry>,
    #[serde(default)]
    pub php: Vec<String>,
    #[serde(default)]
    pub php_default: Option<String>,
    #[serde(default)]
    pub web_default: Option<String>,
    #[serde(default)]
    pub languages: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub language_defaults: std::collections::BTreeMap<String, String>,
}

impl Lock {
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lock::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let s = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, s)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Downloader;

impl Downloader {
    /// Blocking repository download with (optional) checksum verification.
    pub fn download(&self, src: &BinarySource, dest: &Path) -> Result<(), super::error::Error> {
        use sha2::{Digest, Sha256};
        let out = std::process::Command::new("curl")
            .args(["-fsSL", "--", &src.url])
            .output()
            .map_err(|e| super::error::Error::Other(format!("curl exec failed: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(super::error::Error::Other(format!("curl failed: {stderr}")));
        }
        let body = out.stdout;
        let hash = Sha256::digest(&body);
        let hex = format!("{:x}", hash);
        if !src.sha256.is_empty() && src.sha256 != hex {
            return Err(super::error::Error::ChecksumMismatch(src.name.clone()));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if src.tar_gz {
            extract_tarball(&body[..], dest)?;
        } else {
            std::fs::write(dest, body)?;
        }
        Ok(())
    }
}

fn extract_tarball(data: &[u8], dest: &Path) -> Result<(), super::error::Error> {
    use flate2::read::GzDecoder;
    let gz = GzDecoder::new(data);
    let mut tar = tar::Archive::new(gz);
    tar.unpack(dest)?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct PhpManager;

impl PhpManager {
    pub fn installed(&self, lock: &Lock, bin_dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = lock.php.clone();
        for dir in [bin_dir.join("php"), bin_dir.to_path_buf()] {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n.starts_with("php-fpm-") {
                        push_version(&mut v, n.trim_start_matches("php-fpm-"));
                    }
                }
            }
        }
        v.sort();
        v.dedup();
        v
    }

    /// Versions the user dropped as extracted folders under a `php/` dir
    /// (each folder must contain a runnable php-fpm binary).
    pub fn dropped_in(&self, php_root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(php_root) {
            for e in rd.flatten() {
                if !e.path().is_dir() {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_string();
                if php_binary_in(&e.path()).is_some() && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out.sort();
        out
    }
}

/// Version prefix used by `php-fpm-<v>` bin names: "8.4.23" -> "8.4".
pub fn short_version(ver: &str) -> String {
    match ver.find('.') {
        Some(pos) => match ver[pos + 1..].find('.') {
            Some(i) => ver[..pos + 1 + i].to_string(),
            None => ver.to_string(),
        },
        None => ver.to_string(),
    }
}

/// Binary inside an extracted php root dir: `<dir>/php-fpm`,
/// `<dir>/bin/php-fpm` or `<dir>/sbin/php-fpm`.
pub fn php_binary_in(dir: &Path) -> Option<PathBuf> {
    [
        dir.join("php-fpm"),
        dir.join("bin/php-fpm"),
        dir.join("sbin/php-fpm"),
    ]
    .into_iter()
    .find(|c| c.is_file())
}

fn push_version(list: &mut Vec<String>, ver: &str) {
    let short = short_version(ver);
    if !list.contains(&short) {
        list.push(short);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_example_source() {
        let toml = r#"
[[binaries]]
name = "caddy"
version = "2"
url = "https://example.com/caddy.tar.gz"
sha256 = ""
tar_gz = true
files = ["caddy"]
"#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert!(m.find("caddy").unwrap().tar_gz);
    }

    #[test]
    fn php_manager_lists_installed_versions() {
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("php-fpm-8.4"), "").unwrap();
        let lock = Lock::default();
        let pm = PhpManager;
        assert_eq!(pm.installed(&lock, &bin), vec!["8.4".to_string()]);
    }
}
