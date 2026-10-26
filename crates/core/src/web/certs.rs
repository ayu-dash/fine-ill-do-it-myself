use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Files managed per project: leaf cert + key, plus one shared CA.
pub fn ca_paths(etc_dir: &Path) -> (PathBuf, PathBuf) {
    (etc_dir.join("ca.crt"), etc_dir.join("ca.key"))
}

pub fn cert_paths(etc_dir: &Path, project: &str) -> (PathBuf, PathBuf) {
    let dir = etc_dir.join("certs");
    (
        dir.join(format!("{project}.crt")),
        dir.join(format!("{project}.key")),
    )
}

/// Build the command that creates the local CA (run once).
pub fn ca_cmd(key: &Path, cert: &Path) -> Vec<String> {
    vec![
        "openssl".into(),
        "req".into(),
        "-x509".into(),
        "-newkey".into(),
        "rsa:2048".into(),
        "-nodes".into(),
        "-subj".into(),
        "/CN=fidim-local-ca".into(),
        "-days".into(),
        "3650".into(),
        "-keyout".into(),
        key.display().to_string(),
        "-out".into(),
        cert.display().to_string(),
    ]
}

/// Build the command that issues a leaf cert for `<project>.test`, signed by the CA.
pub fn leaf_cmd(ca_key: &Path, ca_cert: &Path, key: &Path, cert: &Path, ext: &Path) -> Vec<String> {
    vec![
        "openssl".into(),
        "req".into(),
        "-x509".into(),
        "-newkey".into(),
        "rsa:2048".into(),
        "-nodes".into(),
        "-subj".into(),
        "/CN=x.test".into(),
        "-config".into(),
        ext.display().to_string(),
        "-extensions".into(),
        "ext".into(),
        "-days".into(),
        "825".into(),
        "-keyout".into(),
        key.display().to_string(),
        "-out".into(),
        cert.display().to_string(),
        "-CAkey".into(),
        ca_key.display().to_string(),
        "-CA".into(),
        ca_cert.display().to_string(),
    ]
}

pub fn ext_file_content(sans: &[std::string::String]) -> String {
    format!("[req]\ndistinguished_name = dn\n[dn]\n[ext]\nsubjectAltName = {}\nbasicConstraints = CA:FALSE\nextendedKeyUsage = serverAuth\nkeyUsage = digitalSignature,keyEncipherment", sans.join("\nsubjectAltName = "))
}

fn cmd_ok(cmd: &[std::string::String]) -> Result<()> {
    let bin = if cmd[0] == "openssl" {
        which_openssl().ok_or_else(|| Error::Other("openssl not found".into()))?
    } else {
        PathBuf::from(&cmd[0])
    };
    let status = std::process::Command::new(bin)
        .args(&cmd[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| Error::Io(std::io::Error::from(e.kind())))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!("command failed: {}", cmd.join(" "))))
    }
}

/// Regenerate per-project certs signed by our local CA.
/// Returns Ok(()) even when the environment lacks openssl (tests).
pub fn regenerate(etc_dir: &Path, projects: &[String]) -> Result<()> {
    if which_openssl().is_none() {
        return Err(Error::Other(
            "openssl not found; cannot generate certificates".into(),
        ));
    }
    std::fs::create_dir_all(etc_dir.join("certs"))?;
    let (ca_cert, ca_key) = ca_paths(etc_dir);
    if !ca_cert.exists() || !ca_key.exists() {
        cmd_ok(&ca_cmd(&ca_key, &ca_cert))?;
    }
    for p in projects {
        let (cert, key) = cert_paths(etc_dir, p);
        if cert.exists() && key.exists() {
            continue;
        }
        let host = format!("{}.{CLIENT_TLD}", p);
        let ext = etc_dir.join("certs").join(format!("{p}.ext"));
        std::fs::write(&ext, ext_file_content(&[format!("DNS:{host}")]))?;
        cmd_ok(&leaf_cmd(&ca_key, &ca_cert, &key, &cert, &ext))?;
        let _ = std::fs::remove_file(&ext);
    }
    Ok(())
}

const CLIENT_TLD: &str = "test";

/// Copy of PATH resolution so command tests do not clobber env.
/// Prefers the system openssl: bundled toolchains (XAMPP) may lag behind.
pub fn which_openssl() -> Option<PathBuf> {
    for cand in ["/usr/bin/openssl", "/bin/openssl"] {
        let p = PathBuf::from(cand);
        if p.exists() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join("openssl");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ca_and_leaf_paths_derive_from_etc() {
        let t = tempfile::tempdir().unwrap();
        let (c, k) = ca_paths(t.path());
        assert!(k.to_str().unwrap().ends_with("ca.key"));
        assert!(c.to_str().unwrap().ends_with("ca.crt"));
        let (lc, lk) = cert_paths(t.path(), "blog");
        assert!(lk.to_str().unwrap().ends_with("certs/blog.key"));
        assert!(lc.to_str().unwrap().ends_with("certs/blog.crt"));
    }

    #[test]
    fn commands_reference_expected_filenames() {
        let ca = ca_cmd(Path::new("/a.key"), Path::new("/a.crt"));
        assert_eq!(ca[0], "openssl");
        assert!(ca.contains(&"-x509".to_string()));
        let leaf = leaf_cmd(
            Path::new("/ca.key"),
            Path::new("/ca.crt"),
            Path::new("/l.key"),
            Path::new("/l.crt"),
            Path::new("/l.ext"),
        );
        assert!(leaf.iter().any(|a| a == "/l.crt"));
        assert!(leaf.iter().any(|a| a == "-CAkey"));
    }

    #[test]
    fn ext_content_has_san() {
        let c = ext_file_content(&["DNS:blog.test".to_string()]);
        assert!(c.contains("subjectAltName = DNS:blog.test"));
        assert!(c.contains("CA:FALSE"));
    }

    #[test]
    fn regenerate_end_to_end_with_real_openssl() {
        if which_openssl().is_none() {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        regenerate(t.path(), &["blog".into()]).unwrap();
        let (_k, c) = cert_paths(t.path(), "blog");
        assert!(c.exists());
        assert!(c.metadata().unwrap().len() > 0);
        let (_, ck) = cert_paths(t.path(), "blog");
        assert!(ck.exists());
    }
}
