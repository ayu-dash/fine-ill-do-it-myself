use assert_cmd::Command;

#[test]
fn creates_projects_under_xdg_data_even_when_run_inside_a_project() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let xdg_data = temp.path().join("xdg-data");
    let working_project = temp.path().join("working-project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&working_project).unwrap();
    std::fs::write(working_project.join("Cargo.toml"), "[package]\n").unwrap();

    let output = Command::cargo_bin("fidim")
        .unwrap()
        .current_dir(&working_project)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &xdg_data)
        .env_remove("FIDIM_DATA_ROOT")
        .env_remove("FIDIM_PROJECT_ROOT")
        .args(["new", "blank-site", "--template", "blank"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(xdg_data
        .join("fine-ill-do-it-myself/projects/blank-site")
        .is_dir());
    assert!(!working_project.join(".fidim").exists());
}

#[test]
fn defaults_to_local_share_when_xdg_data_home_is_unset() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let working_project = temp.path().join("working-project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&working_project).unwrap();
    std::fs::write(working_project.join("Cargo.toml"), "[package]\n").unwrap();

    let output = Command::cargo_bin("fidim")
        .unwrap()
        .current_dir(&working_project)
        .env("HOME", &home)
        .env_remove("XDG_DATA_HOME")
        .env_remove("FIDIM_DATA_ROOT")
        .env_remove("FIDIM_PROJECT_ROOT")
        .args(["new", "blank-site", "--template", "blank"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(home
        .join(".local/share/fine-ill-do-it-myself/projects/blank-site")
        .is_dir());
    assert!(!working_project.join(".fidim").exists());
}

#[test]
fn defaults_to_local_share_when_xdg_data_home_is_empty() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let working_project = temp.path().join("working-project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&working_project).unwrap();

    let output = Command::cargo_bin("fidim")
        .unwrap()
        .current_dir(&working_project)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", "")
        .env_remove("FIDIM_DATA_ROOT")
        .args(["new", "blank-site", "--template", "blank"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(home
        .join(".local/share/fine-ill-do-it-myself/projects/blank-site")
        .is_dir());
}

#[test]
fn next_step_uses_the_project_path_from_the_active_data_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("custom-data-root");
    let output = Command::cargo_bin("fidim")
        .unwrap()
        .arg("--data-root")
        .arg(&root)
        .args(["new", "node-demo", "--template", "node"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!(
            "cd '{}/projects/node-demo' && node server.js",
            root.display()
        )),
        "{stdout}"
    );
}
