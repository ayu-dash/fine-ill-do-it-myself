//! Create a new project under `Sites` from a template.
//!
//! Everything is written into a hidden staging folder (`.<name>.creating`) and
//! renamed into place only after the template succeeded, so a failed download
//! or a missing composer never leaves a half-project behind.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::installer::{self, Fetcher};
use crate::proc::CommandRunner;
use crate::supervisor::Env;

/// WordPress release pinned by `fidim new wordpress`. wordpress.org only
/// publishes SHA-1, so fidim pins its own SHA-256 of the tarball.
pub const WORDPRESS_VERSION: &str = "7.1.3";
pub const WORDPRESS_URL: &str = "https://wordpress.org/wordpress-7.1.3.tar.gz";
pub const WORDPRESS_SHA256: &str =
    "d2a09acb6a15e3b9c471d72557753c266d6f41a79ed19bfe04cbfe49e283b2a5";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    Laravel,
    Wordpress,
    Php,
    Html,
    Blank,
    Node,
    Python,
}

impl Template {
    pub const ALL: [Template; 7] = [
        Template::Laravel,
        Template::Wordpress,
        Template::Php,
        Template::Html,
        Template::Blank,
        Template::Node,
        Template::Python,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Template::Laravel => "laravel",
            Template::Wordpress => "wordpress",
            Template::Php => "php",
            Template::Html => "html",
            Template::Blank => "blank",
            Template::Node => "node",
            Template::Python => "python",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Template::Laravel => "Laravel",
            Template::Wordpress => "WordPress",
            Template::Php => "PHP welcome",
            Template::Html => "Static HTML",
            Template::Blank => "Blank folder",
            Template::Node => "Node (stdlib)",
            Template::Python => "Python (stdlib)",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == value)
    }

    /// Templates that fetch something while creating the project.
    pub fn needs_network(self) -> bool {
        matches!(self, Template::Laravel | Template::Wordpress)
    }

    /// Templates that serve an application process instead of files.
    pub fn proxy_port(self) -> Option<u16> {
        match self {
            Template::Node => Some(3000),
            Template::Python => Some(8000),
            _ => None,
        }
    }

    /// One-line hint shown after creation.
    pub fn next_step(self, project_path: &Path) -> String {
        let project_path = crate::supervisor::shlex_quote(&project_path.display().to_string());
        match self {
            Template::Node => format!("cd {project_path} && node server.js"),
            Template::Python => format!("cd {project_path} && python3 app.py"),
            Template::Laravel => {
                format!("cd {project_path} && php artisan serve  # or just open the site")
            }
            _ => String::new(),
        }
    }
}

/// Project names become DNS labels (`<name>.test`) and folder names, so keep
/// them to lowercase letters, digits and dashes.
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Other("project name must not be empty".into()));
    }
    if name.len() > 63 {
        return Err(Error::Other(
            "project name must be 63 characters or fewer".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(Error::Other(
            "project name may only contain a-z, 0-9 and -".into(),
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(Error::Other(
            "project name must not start or end with -".into(),
        ));
    }
    Ok(())
}

/// Create `<sites>/<name>` from `template`. Returns the new project folder.
pub fn create<F: Fetcher, C: CommandRunner>(
    env: &Env,
    name: &str,
    template: Template,
    fetcher: &F,
    runner: &C,
) -> Result<PathBuf> {
    env.ensure_dirs()?;
    validate_name(name)?;
    let sites = env.paths.sites();
    let final_dir = sites.join(name);
    if final_dir.exists() {
        return Err(Error::Other(format!(
            "project `{name}` already exists at {}",
            final_dir.display()
        )));
    }
    let staging = sites.join(format!(".{name}.creating"));
    let _ = std::fs::remove_dir_all(&staging);
    let result = match template {
        Template::Laravel => create_laravel(env, &staging, runner),
        Template::Wordpress => create_wordpress(&staging, fetcher),
        Template::Php => write_php(&staging),
        Template::Html => write_html(name, &staging),
        Template::Blank => std::fs::create_dir_all(&staging).map_err(Error::Io),
        Template::Node => write_node(name, &staging),
        Template::Python => write_python(&staging),
    };
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    // A fake runner (and composer itself, on some failures) may not create the
    // folder; the rename still has to land on a real directory.
    if !staging.is_dir() {
        std::fs::create_dir_all(&staging)?;
    }
    if let Err(e) = std::fs::rename(&staging, &final_dir) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(Error::Io(e));
    }
    Ok(final_dir)
}

fn create_laravel<C: CommandRunner>(env: &Env, staging: &Path, runner: &C) -> Result<()> {
    if !crate::composer::is_installed(env) {
        return Err(Error::Other(
            "composer is not installed; run `fidim install composer` first".into(),
        ));
    }
    let args = vec![
        "create-project".to_string(),
        "laravel/laravel".to_string(),
        staging.display().to_string(),
        "--no-interaction".to_string(),
        "--prefer-dist".to_string(),
    ];
    let cmd = crate::composer::cmd(env, &args)?;
    runner.run(&cmd, &crate::composer::env_vars(env))?;
    Ok(())
}

fn create_wordpress<F: Fetcher>(staging: &Path, fetcher: &F) -> Result<()> {
    let bytes = fetcher.fetch(WORDPRESS_URL)?;
    if installer::sha256_hex(&bytes) != WORDPRESS_SHA256 {
        return Err(Error::ChecksumMismatch("wordpress".into()));
    }
    // the tarball has a single `wordpress/` top-level folder
    installer::unpack_dir(&bytes, staging, 1)
}

fn write_php(staging: &Path) -> Result<()> {
    std::fs::create_dir_all(staging)?;
    std::fs::write(staging.join("index.php"), php_index())?;
    Ok(())
}

fn write_html(name: &str, staging: &Path) -> Result<()> {
    std::fs::create_dir_all(staging)?;
    std::fs::write(staging.join("index.html"), html_index(name))?;
    Ok(())
}

fn write_node(name: &str, staging: &Path) -> Result<()> {
    std::fs::create_dir_all(staging)?;
    std::fs::write(staging.join("server.js"), NODE_SERVER)?;
    std::fs::write(
        staging.join("package.json"),
        format!(
            r#"{{
  "name": "{name}",
  "private": true,
  "version": "0.0.0",
  "scripts": {{
    "start": "node server.js"
  }}
}}
"#
        ),
    )?;
    std::fs::write(
        staging.join(".fidim.toml"),
        format!(
            "proxy_port = {}\n",
            Template::Node.proxy_port().unwrap_or(3000)
        ),
    )?;
    Ok(())
}

fn write_python(staging: &Path) -> Result<()> {
    std::fs::create_dir_all(staging)?;
    std::fs::write(staging.join("app.py"), PYTHON_APP)?;
    std::fs::write(
        staging.join(".fidim.toml"),
        format!(
            "proxy_port = {}\n",
            Template::Python.proxy_port().unwrap_or(8000)
        ),
    )?;
    Ok(())
}

fn html_index(name: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>{name} — fidim</title>
</head>
<body>
  <h1>{name}</h1>
  <p>Static site created by fidim. Replace <code>index.html</code> with your own.</p>
</body>
</html>
"#
    )
}

fn php_index() -> String {
    r#"<?php
$name = htmlspecialchars(basename(__DIR__), ENT_QUOTES, 'UTF-8');
?>
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title><?= $name ?> — fidim</title>
</head>
<body>
  <h1><?= $name ?></h1>
  <p>PHP <?= PHP_VERSION ?> is running through fidim.</p>
</body>
</html>
"#
    .to_string()
}

const NODE_SERVER: &str = r#"const http = require('http');
const path = require('path');

const port = Number(process.env.PORT || 3000);
const name = path.basename(__dirname);

http
  .createServer((req, res) => {
    res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    res.end(`<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>${name} — fidim</title></head>
<body>
  <h1>${name}</h1>
  <p>Node ${process.version} is running through fidim.</p>
</body>
</html>`);
  })
  .listen(port, '127.0.0.1', () => {
    console.log(`${name} listening on http://127.0.0.1:${port}`);
  });
"#;

const PYTHON_APP: &str = r#"import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get("PORT", "8000"))
NAME = os.path.basename(os.path.dirname(os.path.abspath(__file__)))


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = f"""<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>{NAME} — fidim</title></head>
<body>
  <h1>{NAME}</h1>
  <p>Python is running through fidim.</p>
</body>
</html>""".encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        pass


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn env(t: &tempfile::TempDir) -> Env {
        let env = Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        env
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

    #[derive(Default)]
    struct FakeRunner {
        commands: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
        fail: bool,
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, cmd: &[String], _envs: &[(String, String)]) -> Result<String> {
            self.commands.lock().unwrap().push(cmd.to_vec());
            if self.fail {
                Err(Error::Other("composer failed".into()))
            } else {
                Ok(String::new())
            }
        }
    }

    #[test]
    fn names_must_be_dns_labels() {
        assert!(validate_name("blog").is_ok());
        assert!(validate_name("my-shop-2").is_ok());
        for bad in ["", "Blog", "my_shop", "my.shop", "-blog", "blog-", "a b"] {
            assert!(validate_name(bad).is_err(), "{bad:?} must be rejected");
        }
        assert!(validate_name(&"a".repeat(64)).is_err());
    }

    #[test]
    fn every_template_round_trips_through_its_id() {
        for t in Template::ALL {
            assert_eq!(Template::parse(t.id()), Some(t));
        }
        assert_eq!(Template::parse("nope"), None);
    }

    #[test]
    fn php_template_writes_a_welcome_page() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let f = FakeFetcher::default();
        let r = FakeRunner::default();

        let dir = create(&env, "blog", Template::Php, &f, &r).unwrap();

        assert_eq!(dir, env.paths.sites().join("blog"));
        let index = std::fs::read_to_string(dir.join("index.php")).unwrap();
        assert!(index.contains("<?php"), "{index}");
        assert!(
            f.calls.load(std::sync::atomic::Ordering::SeqCst) == 0,
            "offline template must not fetch"
        );
    }

    #[test]
    fn html_template_writes_an_index() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let dir = create(
            &env,
            "landing",
            Template::Html,
            &FakeFetcher::default(),
            &FakeRunner::default(),
        )
        .unwrap();

        assert!(dir.join("index.html").is_file());
    }

    #[test]
    fn blank_template_only_makes_the_folder() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let dir = create(
            &env,
            "empty",
            Template::Blank,
            &FakeFetcher::default(),
            &FakeRunner::default(),
        )
        .unwrap();

        assert!(dir.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn node_template_writes_a_server_and_proxy_config() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let dir = create(
            &env,
            "app",
            Template::Node,
            &FakeFetcher::default(),
            &FakeRunner::default(),
        )
        .unwrap();

        assert!(dir.join("server.js").is_file());
        assert!(dir.join("package.json").is_file());
        let cfg = std::fs::read_to_string(dir.join(".fidim.toml")).unwrap();
        assert!(cfg.contains("proxy_port = 3000"), "{cfg}");
    }

    #[test]
    fn python_template_writes_a_server_and_proxy_config() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let dir = create(
            &env,
            "api",
            Template::Python,
            &FakeFetcher::default(),
            &FakeRunner::default(),
        )
        .unwrap();

        assert!(dir.join("app.py").is_file());
        let cfg = std::fs::read_to_string(dir.join(".fidim.toml")).unwrap();
        assert!(cfg.contains("proxy_port = 8000"), "{cfg}");
    }

    #[test]
    fn laravel_runs_composer_into_the_staging_folder() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let php = env.paths.bin().join("php/php");
        std::fs::create_dir_all(php.parent().unwrap()).unwrap();
        std::fs::write(&php, "").unwrap();
        std::fs::write(env.paths.bin().join("composer"), "").unwrap();
        let r = FakeRunner::default();

        let dir = create(&env, "shop", Template::Laravel, &FakeFetcher::default(), &r).unwrap();

        let commands = r.commands.lock().unwrap().clone();
        assert_eq!(commands.len(), 1, "{commands:?}");
        let cmd = &commands[0];
        assert!(cmd[0].ends_with("php/php"), "{cmd:?}");
        assert!(cmd.iter().any(|a| a == "create-project"), "{cmd:?}");
        assert!(cmd.iter().any(|a| a == "laravel/laravel"), "{cmd:?}");
        let staging = env.paths.sites().join(".shop.creating");
        assert!(
            cmd.iter().any(|a| a == &staging.display().to_string()),
            "composer must write into the staging dir: {cmd:?}"
        );
        assert_eq!(dir, env.paths.sites().join("shop"));
        assert!(!staging.exists(), "staging folder must be renamed away");
    }

    #[test]
    fn laravel_without_composer_explains_what_to_install() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);

        let err = create(
            &env,
            "shop",
            Template::Laravel,
            &FakeFetcher::default(),
            &FakeRunner::default(),
        )
        .unwrap_err();

        assert!(format!("{err}").contains("composer"), "{err}");
    }

    #[test]
    fn a_failed_template_leaves_no_partial_project() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let php = env.paths.bin().join("php/php");
        std::fs::create_dir_all(php.parent().unwrap()).unwrap();
        std::fs::write(&php, "").unwrap();
        std::fs::write(env.paths.bin().join("composer"), "").unwrap();
        let r = FakeRunner {
            fail: true,
            ..Default::default()
        };

        assert!(create(&env, "shop", Template::Laravel, &FakeFetcher::default(), &r).is_err());

        assert!(!env.paths.sites().join("shop").exists());
        assert!(!env.paths.sites().join(".shop.creating").exists());
    }

    #[test]
    fn an_existing_project_is_never_overwritten() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let existing = env.paths.sites().join("blog");
        std::fs::create_dir_all(&existing).unwrap();
        std::fs::write(existing.join("keep.txt"), "mine").unwrap();

        let err = create(
            &env,
            "blog",
            Template::Blank,
            &FakeFetcher::default(),
            &FakeRunner::default(),
        )
        .unwrap_err();

        assert!(format!("{err}").contains("exists"), "{err}");
        assert_eq!(
            std::fs::read_to_string(existing.join("keep.txt")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn wordpress_rejects_a_checksum_mismatch_without_leaving_files() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        let f = FakeFetcher {
            payload: b"not the wordpress tarball".to_vec(),
            ..Default::default()
        };

        let err = create(&env, "wp", Template::Wordpress, &f, &FakeRunner::default()).unwrap_err();

        assert!(format!("{err}").contains("checksum"), "{err}");
        assert!(!env.paths.sites().join("wp").exists());
        assert!(!env.paths.sites().join(".wp.creating").exists());
    }

    #[test]
    fn template_hints_only_mention_a_command_when_one_is_needed() {
        let node_path = Path::new("/tmp/projects/app");
        let python_path = Path::new("/tmp/projects/api");
        let php_path = Path::new("/tmp/projects/blog");
        assert!(Template::Node
            .next_step(node_path)
            .contains("node server.js"));
        assert!(Template::Python
            .next_step(python_path)
            .contains("python3 app.py"));
        assert!(Template::Php.next_step(php_path).is_empty());
    }

    #[test]
    fn site_detection_uses_the_proxy_config() {
        let t = tempfile::tempdir().unwrap();
        let sites = t.path().join("Sites");
        std::fs::create_dir_all(sites.join("app")).unwrap();
        std::fs::write(sites.join("app/.fidim.toml"), "proxy_port = 3000\n").unwrap();

        let site = crate::site::Site::detect("app", &sites);

        assert_eq!(site.proxy, Some(3000));
    }

    #[test]
    fn site_detection_without_config_has_no_proxy() {
        let t = tempfile::tempdir().unwrap();
        let sites = t.path().join("Sites");
        std::fs::create_dir_all(sites.join("blog")).unwrap();

        assert_eq!(crate::site::Site::detect("blog", &sites).proxy, None);
    }

    #[test]
    fn extract_tree_strips_the_wordpress_folder() {
        let t = tempfile::tempdir().unwrap();
        let staging = t.path().join("staging");
        let bytes = crate::installer::tests_support::tar_gz(&[(
            "wordpress/index.php",
            b"<?php // wordpress".as_slice(),
        )]);

        installer::unpack_dir(&bytes, &staging, 1).unwrap();

        assert!(staging.join("index.php").is_file());
    }

    #[test]
    fn path_helper_is_inside_the_sites_root() {
        let t = tempfile::tempdir().unwrap();
        let env = env(&t);
        assert_eq!(
            env.paths.sites().join("blog"),
            Path::new(&env.paths.sites()).join("blog")
        );
    }
}
