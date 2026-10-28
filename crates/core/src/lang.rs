use std::path::{Path, PathBuf};

use crate::manifest::Lock;
use crate::supervisor::Env;

pub const SUPPORTED_LANGUAGES: &[&str] = &["php", "node", "python", "go", "java"];

/// Probed version from executing a binary or system command (e.g. `node -v`).
pub fn probe_version(cmd: &Path) -> Option<String> {
    let stem = cmd.file_name()?.to_string_lossy().to_string();
    let is_node = stem.starts_with("node");
    let _is_python = stem.starts_with("python");
    let is_go = stem.starts_with("go");
    let _is_java = stem.starts_with("java");
    let is_php = stem.starts_with("php");

    let arg = if is_node || is_php {
        "-v"
    } else if is_go {
        "version"
    } else {
        "--version"
    };

    let out = std::process::Command::new(cmd).arg(arg).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string()
        + " "
        + &String::from_utf8_lossy(&out.stderr);

    parse_version_output(&stem, &text)
}

/// Parse version string from raw tool output.
pub fn parse_version_output(name: &str, text: &str) -> Option<String> {
    let is_node = name.starts_with("node");
    let is_python = name.starts_with("python");
    let is_go = name.starts_with("go");
    let is_java = name.starts_with("java");
    let is_php = name.starts_with("php");

    if is_node {
        for word in text.split_whitespace() {
            let clean = word.trim_start_matches('v');
            if clean.chars().next().is_some_and(|c| c.is_ascii_digit()) && clean.contains('.') {
                return Some(clean.to_string());
            }
        }
    } else if is_python {
        if let Some(pos) = text.find("Python ") {
            let part = &text[pos + 7..];
            let v = part.split_whitespace().next().unwrap_or("").to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    } else if is_go {
        if let Some(pos) = text.find("go version go") {
            let part = &text[pos + 13..];
            let v = part.split_whitespace().next().unwrap_or("").to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    } else if is_java {
        if let Some(start) = text.find('"') {
            if let Some(end) = text[start + 1..].find('"') {
                return Some(text[start + 1..start + 1 + end].to_string());
            }
        }
    } else if is_php {
        if let Some(pos) = text.find("PHP ") {
            let part = &text[pos + 4..];
            let v = part.split_whitespace().next().unwrap_or("").to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Find system executable for the given language on PATH.
pub fn which_system_lang(lang: &str) -> Option<PathBuf> {
    let candidates = match lang {
        "php" => vec!["php"],
        "node" => vec!["node", "nodejs"],
        "python" => vec!["python3", "python"],
        "go" => vec!["go"],
        "java" => vec!["java"],
        other => vec![other],
    };

    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for cand in &candidates {
            let p = dir.join(cand);
            if p.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = p.metadata() {
                        if meta.permissions().mode() & 0o111 != 0 {
                            return Some(p);
                        }
                    }
                }
                #[cfg(not(unix))]
                return Some(p);
            }
        }
    }
    None
}

/// Installed versions for the given language (managed binaries + drop-in folders + system runtime).
pub fn installed_lang_versions(env: &Env, lang: &str) -> Vec<String> {
    if lang == "php" {
        return crate::orchestrator::installed_php_versions(env);
    }
    let mut out = Vec::new();
    let lock = Lock::load(&env.lock_path()).unwrap_or_default();
    if let Some(list) = lock.languages.get(lang) {
        for v in list {
            if !out.contains(v) {
                out.push(v.clone());
            }
        }
    }

    let lang_dir = env.paths.bin().join(lang);
    let prefix = format!("{lang}-");
    if let Ok(rd) = std::fs::read_dir(&lang_dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(v) = n.strip_prefix(&prefix) {
                if !out.contains(&v.to_string()) {
                    out.push(v.to_string());
                }
            }
        }
    }

    // Also check root/<lang> drop-in directory
    let drop_dir = env.paths.root().join(lang);
    if let Ok(rd) = std::fs::read_dir(&drop_dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                let name = e.file_name().to_string_lossy().to_string();
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }

    // Include system runtime if detected on system PATH
    if which_system_lang(lang).is_some() && !out.contains(&"system".to_string()) {
        out.push("system".to_string());
    }

    out.sort();
    out.dedup();
    out
}

/// Resolve the active version of a programming language.
pub fn active_lang_version(env: &Env, lang: &str) -> Option<String> {
    let lock = Lock::load(&env.lock_path()).unwrap_or_default();
    if lang == "php" {
        return Some(crate::orchestrator::default_php(&lock));
    }
    if let Some(v) = lock.language_defaults.get(lang) {
        return Some(v.clone());
    }
    let installed = installed_lang_versions(env, lang);
    if let Some(first) = installed.first() {
        return Some(first.clone());
    }
    // Fall back to system version if present
    if let Some(sys) = which_system_lang(lang) {
        if let Some(ver) = probe_version(&sys) {
            return Some(ver);
        }
        return Some("system".to_string());
    }
    None
}

/// Locate runnable binary for a given language version.
pub fn lang_binary_for(env: &Env, lang: &str, version: &str) -> Option<PathBuf> {
    if lang == "php" {
        return crate::orchestrator::php_binary_for(env, version);
    }
    if version == "system" || version.starts_with("system") {
        return which_system_lang(lang);
    }
    let direct = env.paths.bin().join(lang).join(format!("{lang}-{version}"));
    if direct.is_file() {
        return Some(direct);
    }
    let flat = env.paths.bin().join(format!("{lang}-{version}"));
    if flat.is_file() {
        return Some(flat);
    }
    let drop = env.paths.root().join(lang).join(version);
    for sub in [
        drop.join(lang),
        drop.join(format!("bin/{lang}")),
        drop.join(format!("sbin/{lang}")),
    ] {
        if sub.is_file() {
            return Some(sub);
        }
    }
    if let Some(sys) = which_system_lang(lang) {
        if let Some(sys_ver) = probe_version(&sys) {
            if sys_ver == version || version.contains(&sys_ver) {
                return Some(sys);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_node_version() {
        assert_eq!(
            parse_version_output("node", "v22.11.0\n"),
            Some("22.11.0".to_string())
        );
    }

    #[test]
    fn parse_python_version() {
        assert_eq!(
            parse_version_output("python3", "Python 3.12.3\n"),
            Some("3.12.3".to_string())
        );
    }

    #[test]
    fn parse_go_version() {
        assert_eq!(
            parse_version_output("go", "go version go1.23.2 linux/amd64\n"),
            Some("1.23.2".to_string())
        );
    }

    #[test]
    fn parse_java_version() {
        assert_eq!(
            parse_version_output("java", "openjdk version \"21.0.2\" 2024-01-16\n"),
            Some("21.0.2".to_string())
        );
    }

    #[test]
    fn lang_binary_for_system_and_managed() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::new(t.path().to_path_buf(), t.path().join("sites"));
        env.ensure_dirs().unwrap();

        // System lookup should find node if on PATH (guard against concurrent test PATH clears)
        if let Some(sys) = which_system_lang("node") {
            if let Some(found) = lang_binary_for(&env, "node", "system") {
                assert_eq!(found, sys);
            }
        }

        // Fake a managed binary
        let node_bin_dir = env.paths.bin().join("node");
        std::fs::create_dir_all(&node_bin_dir).unwrap();
        let fake_v20 = node_bin_dir.join("node-20.10.0");
        std::fs::write(&fake_v20, b"#!/bin/sh\n").unwrap();

        assert_eq!(lang_binary_for(&env, "node", "20.10.0"), Some(fake_v20));
        let installed = installed_lang_versions(&env, "node");
        assert!(installed.contains(&"20.10.0".to_string()));
    }
}
