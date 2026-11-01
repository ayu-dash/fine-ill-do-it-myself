//! Shared dashboard asset served by every web server when a request does not
//! match a project, and when no projects exist yet. Keeping it in one module
//! means all servers show the same project list.
use std::path::{Path, PathBuf};

use crate::error::Result;

/// Directory that holds the generated dashboard inside `etc`.
pub fn dir(etc: &Path) -> PathBuf {
    etc.join("dashboard")
}

/// Write `index.html` under `etc/dashboard` and return the dashboard directory.
pub fn write(etc: &Path, projects: &[String], http_port: u16) -> Result<PathBuf> {
    let dir = dir(etc);
    write_to(&dir, projects, http_port)?;
    Ok(dir)
}

/// Write `index.html` into an explicit document root.
pub fn write_to(root: &Path, projects: &[String], http_port: u16) -> Result<()> {
    std::fs::create_dir_all(root)?;
    std::fs::write(root.join("index.html"), generate_html(projects, http_port))?;
    Ok(())
}

/// Simple project list; no CSS or unrelated text on purpose.
pub fn generate_html(projects: &[String], http_port: u16) -> String {
    let port_suffix = if http_port == 80 {
        String::new()
    } else {
        format!(":{http_port}")
    };
    let mut items = String::new();
    for p in projects {
        let url = format!("http://{p}.test{port_suffix}");
        items.push_str(&format!("        <li><a href=\"{url}\">{p}</a></li>\n"));
    }

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Projects</title>
</head>
<body>
    <ul>
{items}    </ul>
</body>
</html>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_each_project_with_its_test_url_and_port() {
        let html = generate_html(&["blog".into(), "shop".into()], 8080);

        assert!(html.contains("http://blog.test:8080"), "{html}");
        assert!(html.contains("http://shop.test:8080"), "{html}");
    }

    #[test]
    fn omits_the_port_suffix_on_port_80() {
        let html = generate_html(&["blog".into()], 80);

        assert!(html.contains("http://blog.test\""), "{html}");
        assert!(!html.contains("blog.test:80"), "{html}");
    }

    #[test]
    fn writes_index_html_into_the_requested_root() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("dashboard");

        write_to(&root, &["blog".into()], 8080).unwrap();

        let html = std::fs::read_to_string(root.join("index.html")).unwrap();
        assert!(html.contains("blog.test"), "{html}");
    }
}
