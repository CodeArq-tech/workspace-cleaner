# Architecture

Git discovers `git-workspace-cleaner` as a subcommand. The Rust executable implements CLI parsing, YAML policy resolution, a JSON registry, Git command execution, and backup and restore.

## Discovery and policy

Global checkout, commit, and merge hooks register activity. Manual registration covers missed and existing worktrees. Records store canonical worktree and common Git paths, observed HEAD and activity time. State is atomically replaced in `~/.local/share/workspace-cleaner`; a lock serializes operations. Rules inherit from broader to narrower paths, with a 14-day default.

`install-path` scans a directory for worktrees and asks for one confirmation before installing repository hooks and registering them. It does not follow symbolic links. The hooks live in the common Git directory and are shared by linked worktrees. Conflicting hooks or custom hooks paths are left untouched and reported; successfully processed worktrees are saved independently.

An external scheduler runs `run --apply`; plain `run` previews. Inactivity combines hook time, HEAD changes and filesystem modification times. Git has no universal post-worktree-create hook; hooks may be bypassed or overridden. A live agent without file changes cannot be detected.

## Backup transaction

Only linked worktrees qualify. A temporary index constructs a tree from HEAD and current non-ignored files, leaving the real index and original branch untouched. `commit-tree` creates a commit, and a push with an empty lease creates a distinct remote ref. The remote ref is read back and compared, then metadata is persisted. HEAD, index and tree are checked again before removing the worktree; the result is persisted. A verified backup remains recorded if removal fails. Staged changes and ignored files cause a skip. The primary worktree is never removed.

External writers can race the final check and removal; this is not a filesystem transaction. Restore fetches the recorded remote branch, compares its commit to the registry, creates a detached linked worktree, and registers it. The local common repository is required; registry metadata is not replicated to the remote. See README for operational limitations.
