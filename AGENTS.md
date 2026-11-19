# Agent instructions

Holt is a Rust Git subcommand for tracking and safely backing up stale linked worktrees. Read ARCHITECTURE.md before changing behavior. All project text, code, documentation, commits, and user-facing messages must be in English.

Preserve user data: never remove a worktree before verifying its remote backup, never rewrite the original branch or index, and fail closed when contents cannot be backed up or the workspace changes. Keep hook and configuration limitations explicit. Test safety-critical behavior with isolated repositories and local bare remotes, never real user workspaces. Run `cargo fmt --check`, `cargo test`, and `cargo clippy` when available.
