use std::path::PathBuf;

/// Disk layout under the application's own directory.
/// `root` is normally `~/.local/share/fine-ill-do-it-myself` but tests
/// inject a temp dir so we never touch the real home directory.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    sites: PathBuf,
}

impl Paths {
    pub fn new(root: PathBuf, sites: PathBuf) -> Self {
        Self { root, sites }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }
    pub fn sites(&self) -> &std::path::Path {
        &self.sites
    }
    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }
    pub fn etc(&self) -> PathBuf {
        self.root.join("etc")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn pids(&self) -> PathBuf {
        self.root.join("run")
    }
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    pub fn runtimes(&self) -> PathBuf {
        self.root.join("runtimes")
    }

    pub fn create_all(&self) -> std::io::Result<()> {
        for d in [
            self.bin(),
            self.etc(),
            self.logs(),
            self.pids(),
            self.data(),
            self.runtimes(),
        ] {
            std::fs::create_dir_all(d)?;
        }
        for sub in [
            "caddy", "nginx", "httpd", "lighttpd", "php", "node", "python", "go", "java",
        ] {
            let d = self.bin().join(sub);
            if !d.is_file() {
                std::fs::create_dir_all(d)?;
            }
        }
        Ok(())
    }
}

/// Find project root by traversing upwards from the current directory,
/// looking for `.fidim`, `Cargo.toml`, or `.git`.
pub fn find_project_root() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("FIDIM_PROJECT_ROOT") {
        return Some(PathBuf::from(explicit));
    }
    let mut cur = std::env::current_dir().ok()?;
    loop {
        if cur.join(".fidim").is_dir()
            || cur.join("Cargo.toml").is_file()
            || cur.join(".git").is_dir()
        {
            return Some(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    std::env::current_dir().ok()
}

/// Default root: `$XDG_DATA_HOME/fine-ill-do-it-myself` (falls back to
/// `~/.local/share/...`). Does not consult HOME when a test passes a custom
/// env probe, so unit tests can stay isolated.
pub fn default_root(data_dir: PathBuf) -> PathBuf {
    data_dir.join("fine-ill-do-it-myself")
}

pub fn default_sites(home: PathBuf) -> PathBuf {
    home.join("Sites")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn builds_layout_under_root() {
        let t = temp();
        let p = Paths::new(t.path().join("app"), t.path().join("Sites"));
        p.create_all().unwrap();
        // A single directory owned by the app holds every artifact.
        assert!(p.bin().exists());
        assert!(p.etc().exists());
        assert!(p.logs().exists());
        assert!(p.pids().exists());
        assert!(p.data().exists());
        assert_eq!(p.sites(), std::path::Path::new(t.path()).join("Sites"));
    }

    #[test]
    fn default_root_uses_xdg_data_dir() {
        let t = temp();
        let root = default_root(t.path().to_path_buf());
        assert!(root.ends_with("fine-ill-do-it-myself"));
    }
}
