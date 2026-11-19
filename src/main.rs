use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};
use walkdir::WalkDir;

#[derive(Parser)]
#[command(
    name = "git-workspace-cleaner",
    version,
    about = "Back up and clean stale Git worktrees"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Install global hooks. Configure a scheduler separately.
    Install,
    /// Find repositories under PATH and install hooks after one confirmation.
    InstallPath { path: PathBuf },
    /// Register the current worktree, or the worktree at PATH.
    Register { path: Option<PathBuf> },
    /// Show registered worktrees. Filter by a project or directory path.
    List {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Show successful remote backups.
    Backups {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Examine registered worktrees; use --apply to back up and remove eligible worktrees.
    Run {
        #[arg(long)]
        apply: bool,
    },
    /// Restore a recorded backup into a new worktree.
    Restore { id: String, path: PathBuf },
    /// Internal hook entry point.
    #[command(hide = true)]
    Hook,
}
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    enabled: bool,
    retention_days: i64,
    remote: String,
    paths: Vec<Rule>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 14,
            remote: "origin".into(),
            paths: vec![],
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    path: String,
    enabled: Option<bool>,
    retention_days: Option<i64>,
}
#[derive(Default, Serialize, Deserialize)]
struct State {
    workspaces: Vec<Workspace>,
    backups: Vec<Backup>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Workspace {
    path: PathBuf,
    common_dir: PathBuf,
    registered_at: DateTime<Utc>,
    last_activity: DateTime<Utc>,
    head: String,
}
#[derive(Serialize, Deserialize)]
struct Backup {
    id: String,
    path: PathBuf,
    common_dir: PathBuf,
    remote: String,
    branch: String,
    commit: String,
    created_at: DateTime<Utc>,
    removed: bool,
}

fn home() -> Result<PathBuf> {
    dirs::home_dir().context("Cannot find home directory")
}
fn state_dir() -> Result<PathBuf> {
    Ok(home()?.join(".local/share/workspace-cleaner"))
}
fn expand(p: &str) -> Result<PathBuf> {
    if p == "~" {
        home()
    } else if let Some(rest) = p.strip_prefix("~/") {
        Ok(home()?.join(rest))
    } else {
        Ok(PathBuf::from(p))
    }
}
fn normalize(p: &Path) -> Result<PathBuf> {
    fs::canonicalize(p).with_context(|| format!("Cannot resolve {}", p.display()))
}
fn config() -> Result<Config> {
    let p = home()?.join(".workspace-cleaner.yaml");
    let cfg: Config = if p.exists() {
        serde_yaml::from_str(&fs::read_to_string(p)?)?
    } else {
        Config::default()
    };
    if cfg.retention_days <= 0
        || cfg
            .paths
            .iter()
            .any(|r| r.retention_days.is_some_and(|d| d <= 0))
    {
        bail!("retention_days must be positive");
    }
    Ok(cfg)
}
fn policy(cfg: &Config, path: &Path) -> Result<(bool, i64)> {
    let mut enabled = cfg.enabled;
    let mut days = cfg.retention_days;
    let mut matches = Vec::new();
    for r in &cfg.paths {
        let p = expand(&r.path)?;
        if !p.is_absolute() {
            bail!("Rule paths must be absolute or start with ~/");
        }
        let p = if p.exists() { normalize(&p)? } else { p };
        if path.starts_with(&p) {
            matches.push((p, r));
        }
    }
    matches.sort_by_key(|(p, _)| p.components().count());
    for (_, r) in matches {
        if let Some(v) = r.enabled {
            enabled = v;
        }
        if let Some(v) = r.retention_days {
            days = v;
        }
    }
    Ok((enabled, days))
}
fn load() -> Result<State> {
    let p = state_dir()?.join("state.json");
    if p.exists() {
        Ok(serde_json::from_slice(&fs::read(p)?)?)
    } else {
        Ok(State::default())
    }
}
fn save(s: &State) -> Result<()> {
    let dir = state_dir()?;
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("state.{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    fs::rename(tmp, dir.join("state.json"))?;
    Ok(())
}
fn git(path: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
fn root(p: &Path) -> Result<PathBuf> {
    normalize(Path::new(&git(p, &["rev-parse", "--show-toplevel"])?))
}
fn register(s: &mut State, p: &Path) -> Result<()> {
    let p = root(p)?;
    let common = git(
        &p,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = normalize(Path::new(&common))?;
    let head = git(&p, &["rev-parse", "HEAD"])?;
    if let Some(w) = s.workspaces.iter_mut().find(|w| w.path == p) {
        w.last_activity = Utc::now();
        w.head = head;
    } else {
        s.workspaces.push(Workspace {
            path: p,
            common_dir: common,
            registered_at: Utc::now(),
            last_activity: Utc::now(),
            head,
        });
    }
    Ok(())
}

fn install_path(path: &Path) -> Result<()> {
    let start = normalize(path)?;
    if !start.is_dir() {
        bail!("Search path must be a directory");
    }
    let mut repos = Vec::new();
    for entry in WalkDir::new(&start)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let entry = entry.context("Could not complete repository search")?;
        if !entry.file_type().is_dir() || !entry.path().join(".git").exists() {
            continue;
        }
        match root(entry.path()) {
            Ok(repo) if repo == normalize(entry.path())? => repos.push(repo),
            Ok(_) => {}
            Err(e) => eprintln!("Skipping {}: {e:#}", entry.path().display()),
        }
    }
    repos.sort();
    repos.dedup();
    if repos.is_empty() {
        println!("No Git worktrees found under {}.", start.display());
        return Ok(());
    }
    println!("Found {} Git worktree(s):", repos.len());
    for repo in &repos {
        println!("  {}", repo.display());
    }
    print!("Install hooks and register these worktrees? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Cancelled; no repositories changed.");
        return Ok(());
    }
    let mut state = load()?;
    let mut failed = 0;
    for repo in repos {
        match install_repository(&repo).and_then(|()| register(&mut state, &repo)) {
            Ok(()) => {
                save(&state)?;
                println!("Installed and registered: {}", repo.display());
            }
            Err(e) => {
                failed += 1;
                eprintln!("Skipped {}: {e:#}", repo.display());
            }
        }
    }
    if failed > 0 {
        bail!("{failed} worktree(s) could not be installed");
    }
    Ok(())
}

fn install_repository(repo: &Path) -> Result<()> {
    git(repo, &["rev-parse", "--verify", "HEAD"]).context("Repository has no valid HEAD")?;
    let common = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let hooks = Path::new(&common).join("hooks");
    let configured = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--get", "core.hooksPath"])
        .output()?;
    if configured.status.success() {
        let value = String::from_utf8_lossy(&configured.stdout)
            .trim()
            .to_owned();
        if Path::new(&value) == state_dir()?.join("hooks") {
            if ["post-checkout", "post-commit", "post-merge"]
                .iter()
                .all(|name| Path::new(&value).join(name).is_file())
            {
                return Ok(());
            }
            bail!("Holt global hooks are incomplete; run install first");
        }
        bail!("core.hooksPath is configured as {value}; refusing to override it");
    }
    if configured.status.code() != Some(1) {
        bail!("Could not inspect core.hooksPath");
    }
    fs::create_dir_all(&hooks)?;
    let names = ["post-checkout", "post-commit", "post-merge"];
    let exe = std::env::current_exe()?;
    let exe = exe
        .to_str()
        .context("Executable path is not UTF-8")?
        .replace('\'', "'\\''");
    let script = format!("#!/bin/sh\n'{exe}' hook >/dev/null 2>&1 || true\n");
    if names
        .iter()
        .all(|name| fs::read_to_string(hooks.join(name)).is_ok_and(|content| content == script))
    {
        return Ok(());
    }
    for name in names {
        if hooks.join(name).exists() {
            bail!("Hook {name} already exists; refusing to overwrite it");
        }
    }
    let mut created = Vec::new();
    let result = (|| -> Result<()> {
        for name in names {
            let file = hooks.join(name);
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)?;
            created.push(file.clone());
            f.write_all(script.as_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(file, fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        for file in created {
            let _ = fs::remove_file(file);
        }
    }
    result
}
fn latest_file_time(path: &Path) -> DateTime<Utc> {
    let mut latest = DateTime::<Utc>::MIN_UTC;
    for e in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .filter_map(|e| e.ok())
    {
        if let Some(t) = e.metadata().ok().and_then(|m| m.modified().ok()) {
            latest = latest.max(DateTime::<Utc>::from(t));
        }
    }
    latest
}
fn slug(s: &str) -> String {
    let v: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if v.is_empty() {
        "unknown".into()
    } else {
        v
    }
}
fn temp_index_command(path: &Path, index: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(path)
        .env("GIT_INDEX_FILE", index)
        .args(args)
        .output()?;
    if !o.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().into())
}
fn snapshot(path: &Path, index: &Path) -> Result<String> {
    if index.exists() {
        fs::remove_file(index)?;
    }
    temp_index_command(path, index, &["read-tree", "HEAD"])?;
    temp_index_command(path, index, &["add", "-A", "--", "."])?;
    temp_index_command(path, index, &["write-tree"])
}
fn backup(s: &mut State, w: &Workspace, cfg: &Config) -> Result<()> {
    let path = &w.path;
    if path.join(".gitmodules").exists() {
        bail!("Submodules require manual handling; skipping");
    }
    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || e.file_name() != ".git")
    {
        let entry = entry.context("Cannot inspect workspace contents")?;
        if entry.depth() > 0 && entry.file_type().is_dir() && entry.path().join(".git").exists() {
            bail!("Nested repository requires manual handling; skipping");
        }
    }
    let listed = git(path, &["worktree", "list", "--porcelain"])?;
    let first = listed
        .lines()
        .next()
        .unwrap_or("")
        .strip_prefix("worktree ")
        .unwrap_or("");
    if normalize(Path::new(first))? == *path {
        bail!("Primary worktree is never removed; skipping");
    }
    if !git(
        path,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )?
    .lines()
    .filter(|l| l.starts_with("!! "))
    .collect::<Vec<_>>()
    .is_empty()
    {
        bail!("Ignored files exist and cannot be included in the backup; skipping");
    }
    if !git(path, &["diff", "--cached", "--name-only"])?.is_empty() {
        bail!("Staged changes require manual handling; skipping to preserve the index");
    }
    let head = git(path, &["rev-parse", "HEAD"])?;
    let branch = git(path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .unwrap_or_else(|_| "detached".into());
    let now = Utc::now();
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("workspace");
    let backup_branch = format!(
        "clean-workspaces-backup-{}-{}-{}-{}",
        slug(&branch),
        slug(name),
        now.format("%Y%m%dT%H%M%SZ"),
        std::process::id()
    );
    let dir = state_dir()?;
    fs::create_dir_all(&dir)?;
    let index = dir.join(format!("index-{}", std::process::id()));
    let result = (|| -> Result<()> {
        let tree = snapshot(path, &index)?;
        let msg = format!("Back up workspace {}", path.display());
        let commit = git(path, &["commit-tree", &tree, "-p", &head, "-m", &msg])?;
        // Never overwrite a pre-existing remote ref, even in the event of a name collision.
        let refspec = format!("{}:refs/heads/{}", commit, backup_branch);
        let lease = format!("--force-with-lease=refs/heads/{}:", backup_branch);
        git(path, &["push", &lease, &cfg.remote, &refspec])?;
        let remote_ref = format!("refs/heads/{}", backup_branch);
        let observed = git(path, &["ls-remote", "--heads", &cfg.remote, &remote_ref])?;
        if observed.split_whitespace().next() != Some(commit.as_str()) {
            bail!("Remote verification failed for {remote_ref}");
        }
        let id = backup_branch.clone();
        s.backups.push(Backup {
            id,
            path: path.clone(),
            common_dir: w.common_dir.clone(),
            remote: cfg.remote.clone(),
            branch: backup_branch.clone(),
            commit: commit.clone(),
            created_at: now,
            removed: false,
        });
        save(s)?;
        if git(path, &["rev-parse", "HEAD"])? != head
            || !git(path, &["diff", "--cached", "--name-only"])?.is_empty()
            || snapshot(path, &index)? != tree
        {
            bail!("Workspace changed during backup; kept in place");
        }
        if git(
            path,
            &[
                "status",
                "--porcelain",
                "--untracked-files=all",
                "--ignored=matching",
            ],
        )?
        .lines()
        .any(|l| l.starts_with("!! "))
        {
            bail!("Ignored files appeared during backup; kept in place");
        }
        git(
            path,
            &[
                "worktree",
                "remove",
                "--force",
                path.to_str().context("Non-UTF-8 path")?,
            ],
        )?;
        s.backups.last_mut().unwrap().removed = true;
        s.workspaces.retain(|x| x.path != *path);
        save(s)?;
        println!(
            "Backed up and removed {} -> {}",
            path.display(),
            backup_branch
        );
        Ok(())
    })();
    let _ = fs::remove_file(index);
    result
}
fn install() -> Result<()> {
    let dir = state_dir()?;
    let hooks = dir.join("hooks");
    fs::create_dir_all(&hooks)?;
    let existing = Command::new("git")
        .args(["config", "--global", "--get", "core.hooksPath"])
        .output()?;
    if existing.status.success()
        && String::from_utf8_lossy(&existing.stdout).trim() != hooks.to_string_lossy()
    {
        bail!("A global core.hooksPath already exists; refusing to replace it");
    }
    let exe = std::env::current_exe()?;
    let text = format!(
        "#!/bin/sh\n\"{}\" hook >/dev/null 2>&1 || true\n",
        exe.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["post-checkout", "post-commit", "post-merge"] {
            let p = hooks.join(name);
            fs::write(&p, &text)?;
            fs::set_permissions(p, fs::Permissions::from_mode(0o755))?;
        }
    }
    let status = Command::new("git")
        .args(["config", "--global", "core.hooksPath"])
        .arg(&hooks)
        .status()?;
    if !status.success() {
        bail!("Could not configure global hooks");
    }
    println!(
        "Global hooks installed at {}. Configure a scheduler to run {} run --apply; see README.md.",
        hooks.display(),
        exe.display()
    );
    Ok(())
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn lock() -> Result<Lock> {
    let dir = state_dir()?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("lock");
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path).with_context(|| format!("Another invocation may be running (lock: {}); if it crashed, remove the lock manually", path.display()))?;
    Ok(Lock(path))
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let _lock = lock()?;
    match cli.command {
        Action::Install => install(),
        Action::InstallPath { path } => install_path(&path),
        Action::Hook => {
            let mut s = load()?;
            register(&mut s, &std::env::current_dir()?)?;
            save(&s)
        }
        Action::Register { path } => {
            let mut s = load()?;
            register(&mut s, &path.unwrap_or(std::env::current_dir()?))?;
            save(&s)
        }
        Action::List { path } => {
            let s = load()?;
            let filter = path.map(|p| normalize(&p)).transpose()?;
            for w in s.workspaces.iter().filter(|w| {
                filter
                    .as_ref()
                    .is_none_or(|p| w.path.starts_with(p) || w.common_dir.starts_with(p))
            }) {
                println!("{}\tlast activity: {}", w.path.display(), w.last_activity);
            }
            Ok(())
        }
        Action::Backups { path } => {
            let s = load()?;
            let filter = path.map(|p| normalize(&p)).transpose()?;
            for b in s.backups.iter().filter(|b| {
                filter
                    .as_ref()
                    .is_none_or(|p| b.path.starts_with(p) || b.common_dir.starts_with(p))
            }) {
                println!(
                    "{}\t{}\t{}\tremoved: {}",
                    b.id,
                    b.path.display(),
                    b.commit,
                    b.removed
                );
            }
            Ok(())
        }
        Action::Run { apply } => {
            let cfg = config()?;
            let mut s = load()?;
            let mut failures = 0;
            for w in s.workspaces.clone() {
                if !w.path.exists() {
                    eprintln!("Missing: {}", w.path.display());
                    continue;
                }
                let (enabled, days) = policy(&cfg, &w.path)?;
                if !enabled {
                    continue;
                }
                let head = match git(&w.path, &["rev-parse", "HEAD"]) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("{}: {e:#}", w.path.display());
                        failures += 1;
                        continue;
                    }
                };
                if head != w.head {
                    if let Some(x) = s.workspaces.iter_mut().find(|x| x.path == w.path) {
                        x.head = head;
                        x.last_activity = Utc::now();
                    }
                    continue;
                }
                let activity = w.last_activity.max(latest_file_time(&w.path));
                if Utc::now().signed_duration_since(activity) < Duration::days(days) {
                    continue;
                }
                println!("Stale: {} (last activity {})", w.path.display(), activity);
                if apply {
                    if let Err(e) = backup(&mut s, &w, &cfg) {
                        eprintln!("{}: {e:#}", w.path.display());
                        failures += 1;
                    }
                }
            }
            save(&s)?;
            if failures > 0 {
                bail!("{failures} workspace(s) failed")
            }
            Ok(())
        }
        Action::Restore { id, path } => {
            let mut s = load()?;
            let b = s
                .backups
                .iter()
                .find(|b| b.id == id)
                .context("Unknown backup ID")?;
            let common = b.common_dir.clone();
            let remote = b.remote.clone();
            let branch = b.branch.clone();
            let commit = b.commit.clone();
            git(
                &common,
                &["fetch", &remote, &format!("refs/heads/{branch}")],
            )?;
            let fetched = git(&common, &["rev-parse", "FETCH_HEAD"])?;
            if fetched != commit {
                bail!("Remote backup no longer matches recorded commit");
            }
            let dest = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()?.join(path)
            };
            git(
                &common,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    dest.to_str().context("Non-UTF-8 path")?,
                    &commit,
                ],
            )?;
            register(&mut s, &dest)?;
            save(&s)?;
            println!("Restored {} at {} (detached HEAD)", id, dest.display());
            Ok(())
        }
    }
}
