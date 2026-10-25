# CLAUDE.md

This file provides guidance to AI coding agents (Claude Code, Codex and others) when working with code in this
repository.

> **`CLAUDE.md` and `AGENTS.md` must stay identical.** Whenever you change one, copy it byte for byte to the
> other in the same change (e.g. `cp CLAUDE.md AGENTS.md`).

# NOBLE — development notes

A HUD terminal workspace written in Rust (edition 2024, 1.95+) with ratatui. See `README.md` for features,
keys and the config schema, and `CONTRIBUTING.md` for the contributor guide.

## Commands
- `cargo test` — unit + headless render + real PTY + end-to-end tests (all must pass)
- `cargo test --test render` — render/PTY tests only; `cargo test --test render <fn_name>` for a single test
- `cargo test --lib <module_or_test_name>` — unit tests inside `src/` only
- `cargo test --test e2e` — runs the compiled binary end to end inside a pseudo-terminal
- `cargo test --test fuzz` — random and hostile input to the outside-data parsers (deterministic; `NOBLE_FUZZ_SEED`, `NOBLE_FUZZ_SCALE`)
- `cargo test --release --test e2e idle -- --ignored --nocapture` — idle CPU/RAM measurement (`#[ignore]`)
- `cargo test --release --test render heavy_output -- --ignored --nocapture` — 50k-line output throughput
- `cargo clippy --all-targets` — kept warning-free
- `cargo fmt` — `rustfmt.toml` (max_width 120)
- `cargo deny check` — RustSec advisories, license allow-list, bans and sources (`deny.toml`; install with
  `cargo install cargo-deny --locked`). A new dependency's license must be added to `deny.toml` after review.
- CI: `.github/workflows/` — `ci.yml` (fmt, clippy `-D warnings` and tests on Windows/Linux/macOS, `shellcheck
  install.sh`, a static linux-musl build, MSRV, cargo-deny), `audit.yml`,
  `release.yml` (`gh workflow run release.yml --ref <branch>` builds without publishing)
- MSRV: `rust-version` in `Cargo.toml` is the oldest Rust the CI `msrv` job builds with; raise it (and the
  README/CONTRIBUTING mentions) when a dependency needs a newer one
- `cargo run --example screenshots` — regenerates the README SVG screenshots
- `cargo run -- --no-boot` — run without the boot animation; `--paths` prints the config/data locations
- `NOBLE_HOME` moves the config and data directory (to experiment without touching the real config)

## Rules
- **Language:** everything is written in **English**: documentation (README, CONTRIBUTING, CHANGELOG, this
  file, workflow and config comments), issue/PR text, commit messages, code identifiers, code comments and UI
  text. This overrides any personal language preference.
- **Dev build — ask first:** after a change, ask the user whether to update the dev build; run it only on a yes.
  On Windows run `cmd /c "%CD%\install.cmd"` (from PowerShell use the full path; the relative name is not found),
  on Linux/macOS `./install.sh`, so the user can open the latest state by typing `noble-dev`. The scripts build in release mode and install
  `~/.cargo/bin/noble-dev(.exe)`; they never touch the stable `noble` and work while noble-dev is running.
  `noble-dev` shows "NOBLE dev" in the top bar and saves its session to `session-dev.json`
  (`util::is_dev_build`); config and data are shared.
- **Windows, Linux and macOS are equal platforms.** Every feature must work on all three, or degrade gracefully
  where the OS lacks something (e.g. no desktop over SSH: `util::has_desktop`). This machine has no Mac or
  Linux: those run in CI only. macOS specifics: login shells by default, the Keychain for AI logins
  (`keychain_has`), Option as Meta for `alt+` keys, `/var` → `/private/var`, case-insensitive paths, `/bin/bash`
  3.2. When touching OS-specific code:
  - Keep platform branches small and side by side (`cfg!(windows)` / `#[cfg(...)]` in the same function), each
    with a comment naming what the other platform does; never leave a platform with a silent no-op.
  - Name things by what they do, not by the OS (paths via `dirs`, programs via `util::which`, `Path` joins —
    no hard-coded `\` or `/`, `.exe` or drive letters outside Windows-only branches).
  - Shells: PowerShell and cmd on Windows; bash (3.2 too), zsh and fish everywhere (Git Bash too); a new shell feature
    needs all of them (`term/pane.rs` `ShellKind`, `term/integration.rs`).
  - Tests that need a real shell or tool run on every platform, choosing the command per OS, and cover every
    installed shell; a platform-specific test returns early on the others with a comment saying why. Every CI
    job must stay green — a change is not done while one fails.
  - User-facing docs (README, config comments, UI text) describe all three platforms.
- README images are generated with `cargo run --example screenshots` (`docs/assets/*.svg`, fake data);
  regenerate them when a screen shown in the README changes. The banner (`docs/assets/banner.svg`) is
  hand-written.
- After a UI change run `cargo test --test render` and review the `target/audit/*.txt` dumps (overflow,
  alignment, small sizes: 160×45 … 30×8).
- Drawing functions write to the `Buffer` only through the `ui/hud.rs` primitives or a bounds-checked
  `buf.cell_mut(..)` (as in `ui/mod.rs`, `ui/overlay.rs`); never index it. `ui::draw` also updates layout state
  (`sync_layout`, `hits`, the slide animation), so it is not a pure function of `App`.
- State only changes on the main thread; background jobs send an `AppEvent`. The one deliberate exception:
  a pane's PTY reader thread feeds the shared `vt100` parser and `Callbacks` and answers terminal queries itself
  (DSR/DA, DECRQM, XTVERSION, OSC 4/10/11/12 colors, `CSI ? 996 n`; queued to the pane's writer thread, like keys).
- Network/CLI errors never panic: providers return `Err(String)`, the UI shows the cache with "~".

## Architecture (parts that span several files)
- **lib + bin split:** all logic lives in the crate under `src/lib.rs`; `main.rs` only does terminal setup,
  panic safety and the frame loop. Tests build `noble::app::App` directly and draw it with `TestBackend` —
  keep new state/screens reachable from tests.
- **Event flow:** input, sensors, project scan, git, the AI collector and a reader/waiter thread per shell send
  `event::AppEvent` over a single `mpsc` channel; `App` (`app/mod.rs`) handles them in the main loop.
  New background job = new `AppEvent` variant + a matching branch in `App`.
- **App submodules:** `app/mod.rs` holds the state types and the event dispatch (`App::handle`, `apply`, `tick`);
  `notify.rs` (toasts, notices, the update notice, PTY output → notifications), `redraw.rs` (animations and the
  next wake-up), `session.rs` (auto-save, restore, shutdown), `power.rs` (battery, clock, sensor rate), `reload.rs`
  (live config reload) and `git.rs` (git status requests) hold the methods on `App`.
- **Redraw (battery-friendly):** there is no fixed frame rate. `App::handle` returns `true` if the event changed
  something visible (e.g. a sensor event while in a terminal returns `false`); anything that changes over time
  (clock, animation, spinner, toast timeout, cursor blink) must be added to `App::redraw_after`, otherwise it
  freezes on screen. Non-event changes are reported via `dirty`/`take_dirty`.
  ≤60 fps under output; an idle terminal redraws once a minute, Home once a second (off battery only).
- **Background work follows what is visible:** sensors via `SensorMode` (System: 1 s including the process
  list, Home: 1 s — 2 s on battery, otherwise 5 s), git status when a command finishes / Home opens / for a
  changed repo, AI quota only while Home is open (`AiReq::Visible`; entering Home fetches at once only if the
  last fetch is older than 30 s, `AiReq::Refresh` — config change, R key — fetches even while Home is hidden).
  Listening ports (every 10 s, 30 s on battery) and the selected project's PR/CI state (60 s, 180 s on battery,
  or at once when its branch changed) are requested only while Home is open (`app/live.rs`).
  The process list refreshes at most every ~1.9 s. On battery (`App::on_battery`, `app/power.rs`) the Home clock has no
  seconds and does not blink.
  When adding periodic work, run it only while the relevant screen is open; measure with
  `cargo test --release --test e2e idle -- --ignored --nocapture` (CPU ms per minute).
- **Effects:** `app/fx.rs` (`App::fx`) holds the short UI effects (fades, flashes, list glides, value tweens).
  Drawing code reports what it sees (`observe_*`) and a change from the previous frame starts an effect;
  `fx.busy()` keeps `App::animating` true until the final frame is drawn. They play only while
  `App::anim_on` (the `animations` setting, and `animations_on_battery` on battery); off, every query
  returns the final state, so render tests turn animations off.
- **Focus mode:** `app/focus.rs`. On a terminal tab with the flag on (`terminal.focus_mode`, prefix f, Settings),
  `App::body()` is the whole screen and `ui::draw` skips both bars. `App::focus_reveal` draws them over the panes
  without a resize: while the prefix is armed, or once the pointer has rested on row 0 or the last row for
  `EDGE_DWELL` (`track_focus_edge` runs on every mouse event, `redraw_after` wakes for it). Other tabs' markers show
  as the `Hit::FocusAlert` chip on the bottom row. A config reload changes the flag only when the file's value changed.
- **Mouse/hit-test:** `ui/*` drawing functions fill `hits: Vec<(Rect, Hit)>` while drawing; `app/input.rs`
  resolves clicks from that list (most recently added first). New clickable element = `app::Hit` variant +
  `hits.push` in drawing + handling in `input.rs`. Hover highlighting is based on the same list.
- **Vendored vt100 and reflow:** the emulator is `src/vt100/` (vt100 0.18.1, MIT, `LICENSE-MIT` kept; owned code,
  it does not follow upstream). `Screen::set_reflow(true)` (set in `Pane::spawn` on every platform) makes
  `Grid::set_size` rewrap the primary grid and its scrollback on a column change (`reflow.rs`; rows carry a `wrapped`
  flag, a wide character moved by the edge leaves a leading spacer); the alternate screen still truncates and pads.
  `set_pull_history(false)` on Windows: ConPTY repaints from its own viewport top (and does not join it to the
  history above), so a taller pane adds blank rows at the bottom instead of pulling history down, a wider screen
  starts a line of its own at its first row, and `Pane::resize` resizes the emulator before the PTY so the repaint
  lands on the reflowed rows. `Screen::history_dropped()` counts lines that scrolled out and
  `reflow_epoch()` changes whenever lines are renumbered; `pane::screen_base` (`history_dropped + scrollback_len`)
  gives a line its stable number, which search matches (`SearchState`, `ScrollText`), OSC 8 spans and
  `scroll_mark` use. On an epoch change search is redone and OSC 8 links are dropped. Reflow is eager (about 100 ms
  at 100k rows, a few ms at the default history).
- **Terminal:** `term/layout.rs` pure split tree (knows nothing about PTYs), `term/pane.rs` PTY + `vt100` +
  shell integration (cwd tracking via OSC 7 / OSC 9;9; the pane's writer thread is the only writer to the PTY
  (`PtyInput`), so `Pane::write` never blocks on a program that does not read; `Pane::cwd` checks a reported folder
  on a background thread (`AppEvent::PaneCwd`), and a network path is shown (`shown_cwd`) but never checked or used),
  `term/osc.rs` the vt100 `Callbacks` (OSC parsing, hyperlinks), `term/input.rs` xterm key/mouse encoding and AltGr
  handling.
- **Shell integration:** PowerShell gets a prompt wrapper (`PWSH_CWD_HOOK`), cmd a `PROMPT` (only when
  `PROMPT` is not already set in NOBLE's environment); bash (`--rcfile`; login arguments use the separate
  `bash_login` script), zsh (`ZDOTDIR`) and fish (`--init-command`) get scripts from `term/integration.rs`,
  written to `<data>/shell/` before a pane starts (`ShellSpec::prepared`). The scripts source the user's own
  config first, then emit OSC 7 on every prompt; Git Bash/Cygwin paths are turned into Windows paths. Arguments
  that replace the config (`--norc`, `-f`, `-N`, `--rcfile`, `-c` …) disable the integration.
- **Clipboard:** `clipboard.rs` keeps one `arboard` handle for the whole run (X11 serves copied text from the
  owning process); without a system clipboard copied text goes out as OSC 52.
- **Terminal queries and modes:** `term/osc.rs` `Callbacks` answers XTVERSION and DECRQM and tracks the DEC modes
  vt100 passes to `unhandled_csi` (`TRACKED_MODES`: 1004 focus reports, 1047, 2026 synchronized output, 2031 color
  notices); `OscScan` (reader thread) finds the terminator of OSC color queries, which vt100 does not pass on.
  `term/colors.rs` `ColorReport` is the palette snapshot (built from `TermPalette::resolve`, shared as
  `App::colors`, written only by the main thread in `App::sync_color_reports`, `app/reports.rs`); a light/dark flip
  sends `CSI ? 997` to panes with 2031. Synchronized output: while a block is open the reader still feeds the parser
  but holds back `PtyOutput` (`Pane::release_sync` from `App::tick` after `SYNC_TIMEOUT` 150 ms); drawing shows
  the pane's copy of its last view (`Pane::remember_view`/`held_view`) meanwhile. Focus reports are sent by
  `App::sync_focus_reports` after every event and action when the focused pane or window focus changes.
- **Notifications to panes:** every pane notification goes through `App::notify` with a `ToastTarget::Pane { tab,
  pane }`; a click (`toast_clicked`) or `⏎` in the notification list (`PaletteCmd::Notice`) switches to it, a closed
  tab or pane only dismisses; `ctrl+y` copies.
- **`noble doctor`:** `doctor.rs` (`gather` probes read-only and bounded, `render` is a pure function of `Facts`),
  run from `main.rs` before the terminal opens; it must stay free of secrets and honor `ai::isolated`.
- **Prompt signal:** `term/osc.rs` `Callbacks` sets the `prompt` flag on every prompt (OSC 7 / 9;9 / 133 A or D);
  `App::on_pty_output` (`app/notify.rs`) treats it as "command finished" → that repo's git status is refreshed with
  `ProjectReq::Refresh`, and a long command in a background tab raises a notification (`notify`).
  OSC 9 text / OSC 777 and the bell put an `alert` (◆) on the tab the same way.
  NOBLE's hooks send `OSC 133;D;<exit code>` before the directory (cmd.exe cannot; the first code of a prompt
  wins) and `OSC 133;C` when a typed command starts (bash 4.4+ `PS0`, zsh `preexec`, fish `fish_preexec`,
  pwsh's wrapped `PSConsoleHostReadLine`; `Pane::start_marks`/`ran`). A command that ran
  (`Pane::command_ran`: the C mark, or for cmd.exe and bash 3.2 a typed line and the last Enter) becomes
  `Pane::last_result`, shown in the pane title (`ui/terminal.rs` `result_badge`), and a running one shows a
  live timer whose redraws `App::redraw_after` schedules (`pane::live_timer_next`).
- **Agents:** `app/agents.rs` holds the hook-record and agent-session logic (`apply_hook_records`,
  `agent_sessions_by_project`, the one-time hooks offer); `app/agent_hooks.rs` holds the per-agent install state
  and the popup toggles; the state types stay in `app/mod.rs`.
- **Session auto-save / termination:** `App::tick` compares the tabs with the last saved session and writes a
  change that lasted 2 s, only while it gets the session lock (`session_save_waiting`, 1 s wait); otherwise the
  save is retried and `session_saved` does not move. Shutdown and the startup restore keep the bounded
  fallback (`session_save`, `session_begin`). Saved tabs that could not start stay in the file (`App::unrestored`).
  `App::shutdown` ends the background commands before it saves; `main.rs` runs the frame loop under `catch_unwind`,
  so a main-thread panic saves (only when the session lock is had within 500 ms; otherwise the last saved file stays) and ends them as well (the panic hook uses `ai::kill_background_children_if_free`,
  which never waits for the registry lock).
  `termination.rs` turns SIGHUP/SIGTERM (Unix) and the console
  close/logoff/shutdown handler (Windows) into `AppEvent::Quit` so the normal shutdown path saves.
- **Outer notifications:** `outer.rs` forwards `notify()` to the outer terminal as OSC 9 / OSC 777 (sanitized;
  skipped while the window is known to be focused; on Windows only when `TERM_PROGRAM` is set).
- **Child processes:** GUI programs (browser, editor, file manager) start through `util::spawn_detached`
  (null stdio, reaper thread on Unix); background git goes through `projects::git_run` (no optional locks,
  no prompts, timeout via `ai::run_command`, which kills the whole tree on every exit; `App::shutdown` ends the
  commands still running with `ai::kill_background_children`, which also closes the registry so none starts after
  it). The codex RPC child (`ai::stdio_rpc`) starts the same way (`ai::spawn_tracked`: group, registry, `Tracked`
  guard). Its `-c` overrides
  (`projects::repo_program_overrides`) turn off `core.fsmonitor`, `log.showSignature` and the repository's own
  `filter.*` commands (read from its `--local`/`--worktree` config only); when that config cannot be read no status is
  shown (fail closed).
- **Untrusted paths:** text from the terminal is never looked at as a network path: `term/osc.rs`
  `is_network_path` (`\\`, `//`) drops an OSC 9;9 report and `term/link.rs` `classify` refuses such tokens before any
  file system call (on Windows that connects and sends NTLM). An OSC 7 `//server/share` (Git Bash) becomes
  `\\server\share` (`osc::windows_cwd`) and is stored, but `Pane::cwd` refuses it like a 9;9 report: the probe never
  sees it, the pane keeps its start folder and `shown_cwd` shows the share. A pane opened in a share keeps it as its
  start folder, so `classify` resolves no relative name against it and git refresh asks for none there.
  `projects::git_dir` does not follow a `.git` file, nor a `commondir` or `gitdir` file in the git directory, that
  names a network path (git itself would read it when it runs in the repository). OSC 52 writes are dropped when
  `terminal.osc52_write` is off (`app/notify.rs`).
- **Search/links:** `app/search.rs` (scrollback search bar, ctrl+click), `term/link.rs` (URL and `file:line`
  detection). Matches store absolute line numbers (0 = oldest scrollback line).
- **Terminal colors:** `theme.rs` `TermScheme`/`TermPalette`; `wt.rs` reads the PowerShell profile's scheme
  and user schemes from Windows Terminal's `settings.json` (JSONC) → `App::term_schemes`. Pane drawing
  (`ui/terminal.rs`) uses `TermPalette::resolve` every frame.
- **Agent hooks:** `hooks/` — `mod.rs` holds what the agents share (`HookRecord` with its optional `agent`, the
  records folder, `run_cli`, `read_records`, `prune`, `remove_record`, `command_base`/`quote_for_shell`, the `Agent`
  registry and `HOOK_ARGS`); one module per agent: `claude.rs` (`~/.claude/settings.json`), `codex.rs`
  (`hooks.json` under `$CODEX_HOME`), `copilot.rs` (its own `noble.json` under `$COPILOT_HOME/hooks`) and
  `opencode.rs` (its own plugin file `plugins/noble.js`). Each offers `EVENTS`, `settings_path` (None when
  `ai::isolated` or no home), `installed_state`, `install`, `uninstall`, `upgrade` and `record_message`; install
  keeps the user's own hooks, writes a backup once and removes only NOBLE's entries. When enabled in Settings
  (`app/agent_hooks.rs`, the "Agent status hooks" popup), an agent's hooks run `noble hook <event> [--agent <id>]`
  (no flag = Claude). `main.rs` handles this subcommand without opening the terminal; state is written to
  `data/agents/<NOBLE_INSTANCE>-<NOBLE_PANE>.json` files, which `App::tick` reads at most once a second
  (`apply_hook_records` → `AgentState`, the record's agent names the pane; `tick` runs on every event and at least
  every `IDLE_TICK` = 2 s, and this scan only while hooks are installed or records exist).
  `session-end` deletes the record, and the prompt signal clears the pane's agent (`App::clear_agent`; older
  records are ignored) and its window title; the launcher command only names the agent until the first prompt
  (`Pane::launch_running`). Running subagents are `<instance>-<pane>.<agent_id>.sub` marker files
  (`SubagentStart`/`SubagentStop`); while any exist the session stays Working after `Stop`. Idle-reminder and
  `agent_completed` notifications are not recorded (`QUIET_NOTIFICATIONS`). `upgrade_installed` (every agent) adds
  newly added events to an older install at startup. `hooks::prune` runs at startup. Never touch the real files in
  tests: install/uninstall/upgrade take an explicit path, and `NOBLE_HOME` does not move `~/.claude`, `~/.codex`,
  `~/.copilot` or `~/.config/opencode`, so e2e sets `NOBLE_NO_SYSTEM_INTEGRATIONS` (`ai::isolated`), which turns
  off every agent's settings path (`settings_path` returns None) and AI credential detection.
- **cmd.exe commands:** the command is passed through the `NOBLE_LAUNCH` environment variable, not as an
  argument (`cmd /K %NOBLE_LAUNCH%`); portable-pty's `\"` escaping breaks quoted paths in cmd.
- **Private files:** on Unix NOBLE's folders are 0700 and new files 0600 (`util::create_private_dir`,
  `util::private_file_options`, `store::write_text`); existing files keep their mode. Windows: the profile's ACL applies.
  `Paths::resolve` returns `None` without a home folder (never a relative `.noble`); `main` reports it.
- **Persistence:** `config.rs` live-reloaded `config.toml` (`App::tick` polls its mtime every 2 s; error =
  toast, never a crash); `store.rs` stores recent dirs, the session, workspaces, AI usage history and UI state
  (`state.json`: welcome seen, pinned, hidden and manually added projects — the scan thread gets the last two as
  `projects::Manual`) with atomic writes (`store::write_bytes`: temp file, `sync_all`, rename, folder fsync on Unix;
  `store::write_json` for JSON, also used for `ai-cache.json`; Windows retries a rename over an open file
  briefly, `store::replace_file`). Unknown top-level fields of `state.json`/`session.json` and workspace entries
  are kept (`extra`). The session file is shared by every window of a build: each window
  (`store::instance_id`) merges only its own tabs in (`session_save`, under a `.lock` file that is removed only
  by its owner), and only the first window of a run restores (`session_begin`, liveness via pid + process start
  time). Settings rewrites of `config.toml` are refused while it does not parse (`config::read_for_settings`).
- **Project live data:** `ports.rs` lists the TCP ports of processes whose current folder is inside a project
  (Linux reads `/proc`; macOS filters with sysinfo and runs `lsof` only for those pids; Windows reads
  `GetExtendedTcpTable` and takes the folders from sysinfo). `pr.rs` runs `gh pr view --json` through
  `ai::run_command` (10 s) and turns every failure (no `gh`, no PR, not signed in, `ai::isolated`) into a silent
  `None`. `app/live.rs` schedules the requests (`request_home_live_data`) and handles `AppEvent::Ports` and
  `AppEvent::Pr`. Tests never run `gh`: `ai::isolated` turns it off in e2e, and headless apps have no `services`,
  so they send no request.
- **Updates:** `update.rs` — `App` asks GitHub's latest release once a day in the background
  (`AppEvent::Update`, result cached in `state.json`; not for `noble-dev`, not when `NOBLE_NO_UPDATE_CHECK`
  is set, as in e2e). A newer version shows a notice at the bottom right (`Hit::Update` opens a tab running
  `noble update`). `noble update` downloads the `release.yml` archive for the platform and its `.sha256`,
  verifies the SHA-256 (a missing checksum file or a mismatch aborts before anything is unpacked, so releases
  without checksums are refused). Only https addresses on GitHub hosts are fetched (`is_release_url`): ureq's own
  redirects are off, `get_following` requests each `Location` itself and checks it, sizes are capped
  (`copy_capped`), and `check_listing` refuses an archive with absolute names, `..` parts or links (`tar -tf`/`-tvf`)
  before `tar` unpacks it. `tar` is run by its full path (`tar_program`: System32 on Windows, else `util::which`). It then swaps the binary: on Linux/macOS one rename over the running binary (no old
  copy is kept); on Windows the running one is renamed to `*.old`, falling back to `.old2` … `.old9` when one
  cannot be deleted, and `update::cleanup_old` removes it on the next launch.

## Extension points
- New action: add it to `Action` in `src/keys.rs` (+ `ALL`, a fixed-size array whose length must be bumped,
  `id`, `title`, `group`), handle it in `App::run` (`src/app/ops.rs`), optionally bind a default key. An action
  that only makes sense inside a pane also goes into the `in_term` list in `palette_items`
  (`src/app/palette.rs`), otherwise it shows up in the palette on every screen.
- New theme: append to `THEMES` in `src/theme.rs` (a fixed-size `[Theme; N]` array: bump `N`) — it appears in
  Settings automatically. The WCAG contrast test in `theme.rs` (`theme_contrast_meets_wcag`) must pass, and the
  theme count ("22 themes") in `config.rs` `DEFAULT_CONFIG` and the README must be updated.
- New AI provider: `detect`/`fetch` + `ProviderDef` in `src/ai/providers.rs`, plus a payload test; also the
  allow-list in `config::parse`, `AiCfg::default`, `DEFAULT_CONFIG` and `app::PROVIDER_KEYS` (the render test
  `every_provider_is_configurable` checks the Settings rows). Tokens only go to their own provider; they are
  never displayed, logged or refreshed.
