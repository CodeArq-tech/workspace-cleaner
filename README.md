# Holt

![An ancient forest shelters two figures from a distant storm as a new green landscape emerges](docs/images/holt-grove.png)

In Norse mythology, Hoddmímis holt is the grove where Líf and Lífþrasir take shelter during Ragnarök, surviving to inhabit the renewed world. Holt borrows that image: it preserves work from an inactive workspace so it can be recovered and continued later. The connection is a metaphor for the project, not a claim that the myth describes a backup. ([Source: an edition of the Poetic Edda](https://library.oapen.org/bitstream/id/1acbfce8-2fa6-468c-a1c8-0fc363011dfa/9781800647749.pdf))

Holt helps developers and AI agents manage Git worktrees that may have been left behind. It tracks registered worktrees, identifies those that appear inactive, and can back up their current changes to a separate remote branch before removing the local worktree. A recorded backup can later be restored into a new worktree.

The default inactivity period is **14 days**. You can change it globally or for specific directories, disable automation for selected paths, or enable it only for an allowlist. The main worktree of a repository is never removed.

Holt is a Git subcommand written in Rust. Run it with `git holt`; it observes some Git activity through hooks and uses an external scheduler for periodic checks. Discovery and inactivity detection are best effort; see [Safety and limitations](#safety-and-limitations) before enabling automatic cleanup.

## Installation

### Prerequisites

- Git and a Rust toolchain with Cargo.
- A configured Git author name and email, and permission to push to the repository's remote.
- The Cargo binary directory (normally `~/.cargo/bin`) on your `PATH` so Git can find `git-holt`.

### Install from source

```sh
git clone https://github.com/CodeArq-tech/workspace-cleaner.git
cd workspace-cleaner
cargo install --locked --path .
git holt -h
```

If upgrading from `git workspace-cleaner`, the configuration and registry remain at their existing paths. Existing hooks and scheduled jobs may still refer to the old executable: keep it available until you update those references. Run `git holt install` to refresh Holt's global hooks if you use them, review repository-specific hooks, and update any existing cron or Task Scheduler command to the new `git-holt` executable. Setup does not replace an existing Holt schedule.

### Initial setup

```sh
git holt setup
```

Setup creates `~/.workspace-cleaner.yaml` with the default 14-day retention period if it does not already exist. It then asks whether to schedule daily cleanup at 03:00 and whether to search a directory for Git worktrees. If you choose a search, enter the path; Holt lists the worktrees and asks for a single confirmation before installing hooks and registering them. Existing configuration is never overwritten.

On macOS and Linux, the scheduler option adds an entry to the current user's crontab, preserving other entries and saving output to `~/.local/share/workspace-cleaner/cleanup.log`. On Windows, it creates a daily task named `Holt Workspace Cleaner` in Task Scheduler. Setup does not replace an existing Holt schedule. A scheduling error is reported; setup still offers repository discovery and returns a failure status afterward. The executable must remain at the installed path, and unattended Git authentication must be configured for backups to succeed.

The scheduled command performs cleanup without another prompt, respecting configured retention, disabled paths and safety checks. Choose `N` if you want to set up scheduling separately. Setup does not install global Git hooks; it installs repository hooks only when a search is confirmed.

To enable automatic registration and activity updates through Git hooks, run:

```sh
git holt install
```

This installs global checkout, commit and merge hooks. It refuses to replace an existing global hooks path. Git uses a single hooks directory: this global setting prevents ordinary repository-local hooks from running, while repositories with their own `core.hooksPath` bypass these hooks. Review existing hooks before installing.

Alternatively, install hooks in existing repositories under a directory with [`install-path`](#find-and-register-worktrees-in-a-directory).

## Usage

### Find and register worktrees in a directory

Run `install-path` with the directory you want to search. It searches that directory and its descendants for Git worktrees, including linked worktrees:

```sh
# Search all projects under ~/Dev.
git holt install-path ~/Dev

# Search a specific directory; quote paths containing spaces.
git holt install-path "$HOME/Dev/My Projects"
```

The path must be an existing directory. Holt lists the discovered worktrees and asks for **one confirmation for the entire list**:

```text
Found 2 Git worktree(s):
  /Users/example/Dev/project-a
  /Users/example/Dev/project-b
Install hooks and register these worktrees? [y/N]
```

Enter `y` or `yes` to proceed. Press Enter or enter any other response to cancel without installing hooks or registering the worktrees. When confirmed, Holt processes each worktree, installs checkout, commit and merge hooks when needed, and saves each successful registration. If Holt's global hooks are already configured and complete, it uses those hooks instead. Verify the result with:

```sh
git holt list
git holt list --path ~/Dev
```

The search does not follow symbolic links or discover bare repositories. Worktrees without a valid `HEAD`, with conflicting hooks, or with another `core.hooksPath` are skipped rather than overwritten. Skips are reported and cause a nonzero exit status; other successful registrations remain saved. Linked worktrees share their repository's hooks. Scanning does not schedule cleanup, and future repositories outside the scan need global hooks or another scan.

### Register and inspect worktrees

Hooks register worktrees they observe, but discovery is best effort: Git has no universal worktree creation hook, hooks can be bypassed, and existing worktrees predate installation. Register existing or missed worktrees manually:

```sh
# Register the current worktree, or pass its path.
git holt register
git holt register /path/to/worktree

# List all registered worktrees or filter by project directory.
git holt list
git holt list --path ~/Dev/MyProject
```

### Preview and clean stale worktrees

```sh
# Preview eligible worktrees without pushing or removing them.
git holt run

# Back up and remove eligible worktrees now.
git holt clean

# Limit the immediate cleanup to a project or directory.
git holt clean --path ~/Dev/MyProject
```

`clean` performs a scan immediately; it still honors retention periods, disabled paths, and all safety checks. It does not force cleanup of recently active worktrees. The path filter matches worktrees under that directory and worktrees belonging to a repository under it. `git holt run --apply` remains an equivalent unfiltered command.

Eligible linked worktrees are pushed and removed without another prompt. Each backup is pushed to a new branch; the original branch and index are not changed. Worktrees that fail the safety checks are skipped.

### Schedule periodic checks

Holt does not run a daemon: Git hooks record activity but do not perform periodic cleanup. Use `git holt setup` to opt into daily scheduling, or configure an external scheduler yourself. For example, add this to your user crontab (`crontab -e`), replacing both absolute paths and ensuring Git and remote credentials work in the scheduler environment:

```cron
0 3 * * * /absolute/path/to/git-holt clean >> /absolute/path/to/workspace-cleaner.log 2>&1
```

This requests a scan each day at 03:00 in the scheduler's local time. Cron does not normally make up a run missed while the computer is asleep or off. Check the log for results and errors; run `git holt clean` manually whenever needed.

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

For example, to disable automation everywhere except one directory:

```yaml
enabled: false
paths:
  - path: ~/Dev/ProjectsToClean
    enabled: true
    retention_days: 7
```

## Backup and recovery

Backups are pushed to a new branch named `clean-workspaces-backup-<branch>-<workspace>-<UTC-date>-<suffix>`. List them and use the displayed backup ID to restore one:

```sh
git holt backups
git holt backups --path ~/Dev/MyProject
git holt restore BACKUP_ID /path/to/new-worktree
cd /path/to/new-worktree
git switch -c my-restored-work
```

Restore fetches and verifies the recorded commit, then creates a detached worktree. Create a new branch there to continue work, as shown above. The original common repository and its configured remote must still exist locally.

The local registry is `~/.local/share/workspace-cleaner/state.json`; back it up independently. It contains metadata, not file contents. After a crash, remove the adjacent `lock` file only after confirming no invocation is running.

## Safety and limitations

Activity is approximated from registration and hook times, changed HEAD, and file modification times outside `.git`. A running agent without file changes, branch creation without checkout, bypassed hooks, or modified timestamps can escape detection. Scans can be expensive. Exclude workspaces that must be retained.

Primary worktrees, worktrees with ignored files or staged changes, and failed or unverifiable pushes are skipped. Submodules and nested repositories cannot be safely represented as ordinary files: do not use automatic cleanup on these projects. Git LFS requires proper remote configuration. Backups may include sensitive untracked files. Concurrent writers can race cleanup; no filesystem transaction guarantees recovery. Review the preview and policies before scheduling.
