# Changelog

All notable changes to NOBLE are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Apps in a pane can get NOBLE's own shortcuts (`alt+m`, `alt+z` …). Prefix `i` locks the pane's keys to its
  app (🔒 KEYS on the pane, prefix `i` again to unlock; the prefix keeps working). With `keys.passthrough =
  "once"` (Settings → Pass shortcuts to apps) prefix + a shortcut sends just that key and prefix `i` the next
  key. Also in the pane menu; new bindable action `passthrough`.

### Changed
- The direct shortcuts that are shell keys (`alt+.` last argument, `alt+t` transpose words, `alt+s` / `alt+p`
  fish's sudo and pager, `alt+,`) now go to the shell in a terminal and stay NOBLE's elsewhere; use the prefix
  there (`ctrl+a :` palette, `n` / `p` tabs). `keys.shell_first = false` (Settings → Leave shell keys to the
  shell) restores the old behavior; a key bound in `direct_bindings` is always NOBLE's.
- Closing a pane or tab (✕, keys, menus) asks first while something still runs in it: a full-screen app, a
  command started at the prompt or a quick-launch command.
- A paste with a line break asks first when the app has no bracketed paste (cmd, older PowerShell), since
  every line could run as a command.

### Fixed
- Restoring a session or workspace no longer drops a tab whose folder cannot be entered, and no longer waits
  on a network share that stopped answering (Windows and Linux): after 2 seconds the pane opens in the home
  folder. A notice names the folders that were not available (also for deleted ones).

## [1.2.0]

### Added
- Keyboard ways to do what needed the mouse: prefix `<` / `>` moves the current tab, prefix `.` opens the pane
  menu (copy path, open folder, VS Code …), and the command palette offers **Update NOBLE** and **Dismiss
  Update Notice** while an update is shown. New bindable actions: `move_tab_left`, `move_tab_right`,
  `pane_menu`, `update`, `dismiss_update`.

### Changed
- The pin (★) and more (⋯) buttons now also show on the selected project, not only under the mouse. Press `→`
  to reach them from the keyboard, `←`/`→` to choose and `⏎` to pin the project or open its menu.

### Fixed
- A `git status` line starting with a non-ASCII character could crash NOBLE while reading a project's changes.
- Dragging a pane divider no longer flickers or lags: the shells are resized once on release instead of at
  every mouse step.
- Dragging a tab over a tab of a different width no longer makes the two swap back and forth.

## [1.1.0]

### Added
- Full Linux support, on par with Windows. bash, zsh and fish (Git Bash on Windows too) now report their
  working directory without any setup: your own `~/.bashrc`, `.zshrc` or `config.fish` loads first, then NOBLE
  adds a small prompt hook. Splits and restored sessions open in the right folder, git status refreshes after
  each command and finished commands in background tabs are marked, as with PowerShell.
- Linux release binaries are static (they run on any distribution) and are also built for ARM64.
- Copy and paste work on Wayland; over SSH or without a system clipboard, copied text reaches your own
  clipboard through the terminal (OSC 52).
- Without a graphical session (SSH, console), links are copied instead of opened, files and the config open in
  a terminal editor, and "open folder" opens a shell tab there.
- Settings offers pwsh and nu on Linux and Git Bash on Windows as shells.
- `install.sh` installs the working tree as `noble-dev` on Linux and macOS.
- Update check: once a day NOBLE looks for a new GitHub release and shows it at the bottom right. Click
  **update** or run `noble update` to download the prebuilt binary and replace the installed one
  (`noble update --check` only reports). Can be turned off in Settings or with `check_updates = false`.

### Fixed
- On X11, copied text could disappear right away when no clipboard manager was running.
- The Antigravity session is now found on Linux (Secret Service), so its quota is shown there too.
- A shell path containing backslashes chosen in Settings is saved as valid TOML.
- Symbols typed with AltGr (`\`, `@`, `{`, `|` … on many European layouts) were dropped in the search bar,
  project and process filters, the command palette and text prompts, so a path could not be typed.
- A window title with certain Unicode letters (e.g. `İ`, the Kelvin sign) or an OSC 7 / `file://` link with a
  `%` before a non-ASCII character could crash NOBLE.
- `settings.json` files starting with a UTF-8 BOM (as written by PowerShell 5 or Notepad) are now read; the
  Claude hooks setting no longer fails on them.
- Paths next to the home folder that share its name (`C:\Users\me2`) are no longer shortened to `~`.
- Turning the Claude hooks on or off keeps the key order of `~/.claude/settings.json`.
- The Claude Code hooks setting is hidden when Claude Code is not installed (it stays visible while the
  hooks are still set, so they can be removed).

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

[Unreleased]: https://github.com/MertSoylu/noble/compare/v1.2.0...HEAD
[1.2.0]: https://github.com/MertSoylu/noble/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/MertSoylu/noble/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/MertSoylu/noble/releases/tag/v1.0.0
