use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn cmd(program: &str, args: &[&str], dir: &Path, home: &Path) -> String {
    let o = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}
fn git(args: &[&str], dir: &Path, home: &Path) -> String {
    cmd("git", args, dir, home)
}
fn cli(args: &[&str], dir: &Path, home: &Path) -> String {
    cmd(env!("CARGO_BIN_EXE_git-holt"), args, dir, home)
}
#[test]
fn backup_and_restore_preserve_changes() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("integration-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let home = base.join("home");
    let repo = base.join("repo");
    let remote = base.join("remote.git");
    let work = base.join("work");
    let restored = base.join("restored");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&repo).unwrap();
    git(&["init", "--bare", remote.to_str().unwrap()], &base, &home);
    git(&["init"], &repo, &home);
    git(&["config", "user.name", "Test"], &repo, &home);
    git(&["config", "user.email", "test@example.com"], &repo, &home);
    fs::write(repo.join("file"), "original\n").unwrap();
    git(&["add", "file"], &repo, &home);
    git(&["commit", "-m", "initial"], &repo, &home);
    git(
        &["remote", "add", "origin", remote.to_str().unwrap()],
        &repo,
        &home,
    );
    git(
        &["worktree", "add", "-b", "feature", work.to_str().unwrap()],
        &repo,
        &home,
    );
    fs::write(work.join("file"), "changed\n").unwrap();
    fs::write(work.join("new"), "untracked\n").unwrap();
    cli(&["register", work.to_str().unwrap()], &repo, &home);
    cli(&["clean"], &repo, &home);
    assert!(work.exists(), "recent worktrees must not be cleaned");
    let state_path = home.join(".local/share/workspace-cleaner/state.json");
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    state["workspaces"][0]["last_activity"] = "2000-01-01T00:00:00Z".into();
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    for p in [&work, &work.join("file"), &work.join("new")] {
        cmd(
            "touch",
            &["-t", "200001010000", p.to_str().unwrap()],
            &base,
            &home,
        );
    }
    assert!(cli(&["run"], &repo, &home).contains("Stale:"));
    assert!(work.exists());
    cli(&["clean", "--path", home.to_str().unwrap()], &repo, &home);
    assert!(work.exists(), "worktrees outside the filter must be kept");
    cli(&["clean", "--path", repo.to_str().unwrap()], &repo, &home);
    assert!(!work.exists());
    let state: serde_json::Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    let id = state["backups"][0]["id"].as_str().unwrap();
    assert_eq!(state["backups"][0]["removed"], true);
    cli(&["restore", id, restored.to_str().unwrap()], &repo, &home);
    assert_eq!(
        fs::read_to_string(restored.join("file")).unwrap(),
        "changed\n"
    );
    assert_eq!(
        fs::read_to_string(restored.join("new")).unwrap(),
        "untracked\n"
    );
    assert_eq!(fs::read_to_string(repo.join("file")).unwrap(), "original\n");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn install_path_asks_once_and_preserves_existing_hooks() {
    use std::io::Write;
    use std::process::Stdio;
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("discovery-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let home = base.join("home");
    let projects = base.join("projects");
    let repo = projects.join("first");
    let second = projects.join("second");
    for p in [&home, &repo, &second] {
        fs::create_dir_all(p).unwrap();
    }
    for p in [&repo, &second] {
        git(&["init"], p, &home);
        git(&["config", "user.name", "Test"], p, &home);
        git(&["config", "user.email", "test@example.com"], p, &home);
        fs::write(p.join("file"), "initial").unwrap();
        git(&["add", "file"], p, &home);
        git(&["commit", "-m", "initial"], p, &home);
    }
    let existing = second.join(".git/hooks/post-commit");
    fs::write(&existing, "existing hook\n").unwrap();
    let invoke = |answer: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_git-holt"))
            .args(["install-path", projects.to_str().unwrap()])
            .current_dir(&base)
            .env("HOME", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(answer.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let no = invoke("n\n");
    assert!(no.status.success());
    let out = String::from_utf8_lossy(&no.stdout);
    assert!(out.contains("first") && out.contains("second"));
    assert_eq!(out.matches("[y/N]").count(), 1);
    assert!(!repo.join(".git/hooks/post-commit").exists());
    assert!(!home
        .join(".local/share/workspace-cleaner/state.json")
        .exists());
    let mut setup = Command::new(env!("CARGO_BIN_EXE_git-holt"))
        .arg("setup")
        .current_dir(&base)
        .env("HOME", &home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    write!(
        setup.stdin.take().unwrap(),
        "no\nyes\n{}\nyes\n",
        projects.display()
    )
    .unwrap();
    let yes = setup.wait_with_output().unwrap();
    assert!(!yes.status.success());
    assert!(String::from_utf8_lossy(&yes.stdout).contains("Found 2 Git worktree(s)"));
    assert!(repo.join(".git/hooks/post-commit").exists());
    assert_eq!(fs::read_to_string(existing).unwrap(), "existing hook\n");
    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(home.join(".local/share/workspace-cleaner/state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["workspaces"].as_array().unwrap().len(), 1);
    let _ = fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn setup_preserves_config_and_installs_only_one_cron_entry() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("setup-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let home = base.join("home");
    let bin = base.join("bin");
    let capture = base.join("crontab");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fs::write(&capture, "15 8 * * * existing-job\n").unwrap();
    let mock = bin.join("crontab");
    fs::write(&mock, "#!/bin/sh\nif [ \"$1\" = -l ]; then cat \"$TEST_CRON_CAPTURE\"; else cat > \"$TEST_CRON_CAPTURE\"; fi\n").unwrap();
    fs::set_permissions(&mock, fs::Permissions::from_mode(0o755)).unwrap();
    let invoke = |answers: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_git-holt"))
            .arg("setup")
            .current_dir(&base)
            .env("HOME", &home)
            .env("TEST_CRON_CAPTURE", &capture)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(answers.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let first = invoke("yes\nno\n");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let config = home.join(".workspace-cleaner.yaml");
    assert!(fs::read_to_string(&config)
        .unwrap()
        .contains("retention_days: 14"));
    let scheduled = fs::read_to_string(&capture).unwrap();
    assert!(scheduled.contains("15 8 * * * existing-job"));
    assert!(scheduled.contains("0 3 * * * PATH="));
    assert!(scheduled.contains(" clean >> "));
    fs::write(&config, "retention_days: 30\n").unwrap();
    let second = invoke("yes\nno\n");
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("already exists"));
    assert_eq!(fs::read_to_string(&capture).unwrap(), scheduled);
    assert_eq!(fs::read_to_string(&config).unwrap(), "retention_days: 30\n");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn git_discovers_holt_subcommand() {
    let executable = Path::new(env!("CARGO_BIN_EXE_git-holt"));
    let mut paths = vec![executable.parent().unwrap().to_path_buf()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let output = Command::new("git")
        .args(["holt", "-h"])
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("git-holt"));
}
