# Changelog

All notable changes to NOBLE are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Update check: once a day NOBLE looks for a new GitHub release and shows it at the bottom right. Click
  **update** or run `noble update` to download the prebuilt binary and replace the installed one
  (`noble update --check` only reports). Can be turned off in Settings or with `check_updates = false`.

## [1.0.0]

First public release.

### Terminals
- Real shells in tabs and splits (ConPTY on Windows): zoom, drag to resize, drag tabs to reorder, rename.
- Scrollback search, `ctrl+click` on URLs, OSC 8 hyperlinks and `file:line` paths.
- Working-directory tracking through OSC 7 / OSC 9;9 for PowerShell, cmd and OSC 7 prompts.
- Background tabs are marked when a long command finishes, the bell rings or an app sends a notification.
- Session restore and named workspaces.

### Home
- Git projects with branch, changes, commits to push or pull, and a card with recent commits and changed files.
- Quick launch for 13 AI CLIs (Claude Code, Codex, OpenCode, Copilot, Antigravity, Pi, oh-my-pi, Freebuff,
  Grok Build, Cursor, Command Code, Cline, Kilo Code). Only installed ones are shown, and each can be hidden or
  rebound in Settings.
- AI quota for Claude Code, Codex, Antigravity, OpenCode Go, Kilo Code and Command Code, with a pace warning.
- Optional Claude Code hooks that show each session as working, needs you or your turn.

### System
- CPU history and per-core load, memory, network, disks, battery and a sortable process table.

### Settings
- 20 themes, terminal color schemes read from Windows Terminal, live-reloaded `config.toml`.

### Development
- `install.cmd` installs the working tree as `noble-dev`, next to a stable `noble`.
- `cargo run --example screenshots` regenerates the README images.

[Unreleased]: https://github.com/MertSoylu/noble/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/MertSoylu/noble/releases/tag/v1.0.0
