# Workspace Cleaner

A Git subcommand that records, reviews, backs up, and cleans stale linked Git worktrees.

## Installation

Install Rust and Git, then run `cargo install --locked --path .` and `git workspace-cleaner install`. The latter installs global checkout, commit and merge hooks. It refuses to replace an existing global hooks path. Git uses a single hooks directory: this global setting prevents ordinary repository-local hooks from running, while repositories with their own `core.hooksPath` bypass these hooks. Review existing hooks before installing.

Discovery is best effort: Git has no universal worktree creation hook, hooks can be bypassed, and existing worktrees predate installation. Use `git workspace-cleaner register /path/to/worktree` for missed worktrees. `git workspace-cleaner list` lists registered paths; `--path /project` filters results.

Schedule the absolute executable path with your preferred scheduler. For example, add this to your user crontab, replacing both absolute paths and ensuring Git and remote credentials work in the scheduler environment:

```cron
0 3 * * * /absolute/path/to/git-workspace-cleaner run --apply >> /absolute/path/to/workspace-cleaner.log 2>&1
```

`git workspace-cleaner run` previews candidates. With `--apply`, eligible linked worktrees are pushed and removed without another prompt.

## Configuration

Create `~/.workspace-cleaner.yaml`. Every field is optional:

```yaml
enabled: true
retention_days: 14
remote: origin
paths:
  - path: ~/Dev
    retention_days: 7
  - path: ~/Dev/Pontotel
    retention_days: 30
  - path: ~/Dev/Pontotel/keep
    enabled: false
```

More specific path rules override individual settings of broader rules. Set global `enabled: false` and selectively enable paths to use an allowlist. Paths must be absolute or start with `~/`; retention must be positive days.

## Backup and recovery

Backups are pushed to a new branch named `clean-workspaces-backup-<branch>-<workspace>-<UTC-date>-<suffix>`. List them with `git workspace-cleaner backups` or filter with `--path /project`. Run `git workspace-cleaner restore BACKUP_ID /path/to/new-worktree` to fetch and verify a recorded commit and create a detached worktree; create a new branch there to continue work. The original common repository and its configured remote must still exist locally.

The local registry is `~/.local/share/workspace-cleaner/state.json`; back it up independently. It contains metadata, not file contents. After a crash, remove the adjacent `lock` file only after confirming no invocation is running.

## Safety and limitations

Activity is approximated from registration and hook times, changed HEAD, and file modification times outside `.git`. A running agent without file changes, branch creation without checkout, bypassed hooks, or modified timestamps can escape detection. Scans can be expensive. Exclude workspaces that must be retained.

Primary worktrees, worktrees with ignored files or staged changes, and failed or unverifiable pushes are skipped. Submodules and nested repositories cannot be safely represented as ordinary files: do not use automatic cleanup on these projects. Git LFS requires proper remote configuration. Backups may include sensitive untracked files. Concurrent writers can race cleanup; no filesystem transaction guarantees recovery. Review the preview and policies before scheduling.
