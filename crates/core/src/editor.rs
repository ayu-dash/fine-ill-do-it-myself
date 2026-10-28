//! Detect the code editor used by the TUI (`o` on a project row) and the
//! command line to launch it.
//!
//! GUI editors are spawned detached so they never touch the terminal the TUI
//! owns. Terminal editors must be run attached, after the TUI has given the
//! screen back; the `kind` field tells the caller which is which.

use std::path::{Path, PathBuf};

/// How the editor takes over the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKind {
    /// Own window; safe to spawn in the background.
    Gui,
    /// Runs inside the terminal, so the TUI has to suspend itself.
    Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editor {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub kind: EditorKind,
}

impl Editor {
    /// Full argv for opening `path` (the path always comes last).
    pub fn command(&self, path: &Path) -> Vec<String> {
        let mut cmd = vec![self.program.display().to_string()];
        cmd.extend(self.args.iter().cloned());
        cmd.push(path.display().to_string());
        cmd
    }

    pub fn name(&self) -> String {
        self.program
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.program.display().to_string())
    }
}

/// Tried in order after the environment variables, GUI first so the TUI keeps
/// the terminal it owns.
pub const GUI_EDITORS: [&str; 8] = [
    "code", "codium", "cursor", "zed", "subl", "idea", "kate", "geany",
];
pub const TERMINAL_EDITORS: [&str; 5] = ["nvim", "vim", "nano", "hx", "micro"];

/// Environment variables honoured, highest priority first.
pub const EDITOR_ENV_VARS: [&str; 3] = ["FIDIM_EDITOR", "VISUAL", "EDITOR"];

/// Pick the editor to use. `lookup` resolves a program name (or path) to an
/// executable and `getenv` reads the environment, so this is testable without
/// touching the real process environment.
pub fn detect(
    lookup: &dyn Fn(&str) -> Option<PathBuf>,
    getenv: &dyn Fn(&str) -> Option<String>,
) -> Option<Editor> {
    // explicit configuration first, so the user can override detection
    for key in EDITOR_ENV_VARS {
        let Some(value) = getenv(key) else { continue };
        let mut parts = value.split_whitespace();
        let Some(program) = parts.next() else {
            continue;
        };
        let Some(path) = lookup(program) else {
            continue;
        };
        return Some(Editor {
            kind: kind_of(program),
            program: path,
            args: parts.map(str::to_string).collect(),
        });
    }
    // then the editors we know, GUI before terminal
    GUI_EDITORS
        .iter()
        .chain(TERMINAL_EDITORS.iter())
        .find_map(|name| {
            lookup(name).map(|program| Editor {
                kind: kind_of(name),
                program,
                args: Vec::new(),
            })
        })
}

/// Terminal editors need the TUI to hand the screen over; everything else
/// (including a custom `FIDIM_EDITOR`) is assumed to open its own window.
fn kind_of(program: &str) -> EditorKind {
    let base = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| program.to_string());
    if TERMINAL_EDITORS.contains(&base.as_str()) {
        EditorKind::Terminal
    } else {
        EditorKind::Gui
    }
}

/// Detect with the real environment and `PATH`.
pub fn detect_default() -> Option<Editor> {
    detect(&which, &|key| std::env::var(key).ok())
}

/// Resolve `name` through `$PATH`; a name containing `/` is checked directly.
pub fn which(name: &str) -> Option<PathBuf> {
    which_in(std::env::var("PATH").ok().as_deref(), name)
}

/// Pure `PATH` search used by `which` (and by tests).
pub fn which_in(path_var: Option<&str>, name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return is_executable(&p).then_some(p);
    }
    for dir in path_var.unwrap_or_default().split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(names: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
        move |name: &str| {
            names
                .iter()
                .find(|n| **n == name)
                .map(|n| PathBuf::from(format!("/usr/bin/{n}")))
        }
    }

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn fidim_editor_env_wins_over_visual_and_editor() {
        let e = detect(
            &found(&["myed", "vim", "nano"]),
            &env(&[
                ("FIDIM_EDITOR", "myed"),
                ("VISUAL", "vim"),
                ("EDITOR", "nano"),
            ]),
        )
        .expect("an editor must be detected");

        assert_eq!(e.program, PathBuf::from("/usr/bin/myed"));
    }

    #[test]
    fn visual_beats_editor() {
        let e = detect(
            &found(&["vim", "nano"]),
            &env(&[("VISUAL", "vim"), ("EDITOR", "nano")]),
        )
        .expect("an editor must be detected");

        assert_eq!(e.program, PathBuf::from("/usr/bin/vim"));
    }

    #[test]
    fn editor_env_is_used_when_visual_is_missing() {
        let e = detect(&found(&["nano"]), &env(&[("EDITOR", "nano")]))
            .expect("an editor must be detected");

        assert_eq!(e.program, PathBuf::from("/usr/bin/nano"));
    }

    #[test]
    fn env_value_can_carry_extra_args() {
        let e = detect(
            &found(&["code"]),
            &env(&[("FIDIM_EDITOR", "code --reuse-window")]),
        )
        .expect("an editor must be detected");

        assert_eq!(e.program, PathBuf::from("/usr/bin/code"));
        assert_eq!(e.args, vec!["--reuse-window".to_string()]);
    }

    #[test]
    fn unknown_env_program_falls_back_to_detection() {
        let e = detect(
            &found(&["zed"]),
            &env(&[("FIDIM_EDITOR", "no-such-editor")]),
        )
        .expect("detection must still find zed");

        assert_eq!(e.program, PathBuf::from("/usr/bin/zed"));
    }

    #[test]
    fn empty_env_value_is_ignored() {
        let e = detect(&found(&["code"]), &env(&[("FIDIM_EDITOR", "   ")]))
            .expect("detection must still find code");

        assert_eq!(e.program, PathBuf::from("/usr/bin/code"));
    }

    #[test]
    fn gui_editors_are_preferred_over_terminal_ones() {
        let e =
            detect(&found(&["code", "zed", "vim"]), &env(&[])).expect("an editor must be detected");

        assert_eq!(e.program, PathBuf::from("/usr/bin/code"));
        assert_eq!(e.kind, EditorKind::Gui);
    }

    #[test]
    fn terminal_editors_are_marked_as_such() {
        let e = detect(&found(&["vim"]), &env(&[])).expect("vim must be detected");

        assert_eq!(e.kind, EditorKind::Terminal);
    }

    #[test]
    fn absolute_env_path_is_accepted() {
        let e = detect(
            &|name: &str| (name == "/opt/ed/bin/zed").then(|| PathBuf::from(name)),
            &env(&[("FIDIM_EDITOR", "/opt/ed/bin/zed")]),
        )
        .expect("absolute path must be accepted");

        assert_eq!(e.program, PathBuf::from("/opt/ed/bin/zed"));
        assert_eq!(e.kind, EditorKind::Gui);
    }

    #[test]
    fn no_editor_returns_none() {
        assert!(detect(&found(&[]), &env(&[])).is_none());
    }

    #[test]
    fn command_puts_the_project_path_last() {
        let e = Editor {
            program: PathBuf::from("/usr/bin/code"),
            args: vec!["--reuse-window".into()],
            kind: EditorKind::Gui,
        };

        assert_eq!(
            e.command(Path::new("/home/me/Sites/blog")),
            vec![
                "/usr/bin/code".to_string(),
                "--reuse-window".to_string(),
                "/home/me/Sites/blog".to_string()
            ]
        );
    }

    #[test]
    fn which_in_finds_executables_on_the_given_path() {
        let t = tempfile::tempdir().expect("tempdir");
        let bin = t.path().join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        let tool = bin.join("myed");
        std::fs::write(&tool, "#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let path_var = bin.display().to_string();

        assert_eq!(which_in(Some(&path_var), "myed"), Some(tool));
        assert_eq!(which_in(Some(&path_var), "missing"), None);
    }

    #[test]
    fn which_in_accepts_a_direct_path() {
        let t = tempfile::tempdir().expect("tempdir");
        let tool = t.path().join("myed");
        std::fs::write(&tool, "#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }

        assert_eq!(
            which_in(None, tool.to_str().expect("utf8 path")),
            Some(tool.clone())
        );
        assert_eq!(which_in(None, "relative/not/here"), None);
    }
}
