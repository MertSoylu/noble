# CLAUDE.md

This file provides guidance to AI coding agents (Claude Code, Codex and others) when working with code in this
repository.

> **`CLAUDE.md` and `AGENTS.md` must stay identical.** Whenever you change one, copy it byte for byte to the
> other in the same change (e.g. `cp CLAUDE.md AGENTS.md`).

# NOBLE — development notes

A HUD terminal workspace written in Rust (edition 2024, 1.88+) with ratatui. See `README.md` for features,
keys and the config schema, and `CONTRIBUTING.md` for the contributor guide.

## Commands
- `cargo test` — unit + headless render + real PTY + end-to-end tests (all must pass)
- `cargo test --test render` — render/PTY tests only; `cargo test --test render <fn_name>` for a single test
- `cargo test --lib <module_or_test_name>` — unit tests inside `src/` only
- `cargo test --test e2e` — runs the compiled binary end to end inside a pseudo-terminal
- `cargo test --release --test e2e idle -- --ignored --nocapture` — idle CPU/RAM measurement (`#[ignore]`)
- `cargo test --release --test render heavy_output -- --ignored --nocapture` — 50k-line output throughput
- `cargo clippy --all-targets` — kept warning-free
- `cargo fmt` — `rustfmt.toml` (max_width 120)
- CI: `.github/workflows/ci.yml` (Windows: fmt + clippy `-D warnings` + all tests; Linux: clippy + `--lib`),
  `release.yml` (Windows/Linux binaries on a `v*` tag)
- `cargo run --example screenshots` — regenerates the README SVG screenshots
- `cargo run -- --no-boot` — run without the boot animation; `--paths` prints the config/data locations
- `NOBLE_HOME` moves the config and data directory (to experiment without touching the real config)

## Rules
- **Language:** all documentation (README, CONTRIBUTING, CHANGELOG, this file, workflow and config comments,
  issue/PR text) and all **commit messages** are written in **English**. This overrides any personal preference
  for Turkish commit messages. Code identifiers and UI text are English; code comments are currently Turkish
  and new comments follow the surrounding code.
- **After every change, update the dev build:** run `cmd /c "%CD%\install.cmd"` (from PowerShell use the full
  path; the relative name is not found) so the user can open the latest state by typing `noble-dev`. This step
  is never skipped. The script builds in release mode and installs `~/.cargo/bin/noble-dev.exe`; it never
  touches the stable `noble`. It renames a running noble-dev.exe out of the way first. `noble-dev` shows
  "NOBLE dev" in the top bar and saves its session to `session-dev.json` (`util::is_dev_build`); config and
  data are shared.
- README images are generated with `cargo run --example screenshots` (`docs/assets/*.svg`, fake data);
  regenerate them when a screen shown in the README changes. The banner (`docs/assets/banner.svg`) is
  hand-written.
- After a UI change run `cargo test --test render` and review the `target/audit/*.txt` dumps (overflow,
  alignment, small sizes: 160×45 … 30×8).
- Drawing functions only use the `ui/hud.rs` primitives (bounds-safe); never write to the `Buffer` by index.
- State only changes on the main thread; background jobs send an `AppEvent`.
- Network/CLI errors never panic: providers return `Err(String)`, the UI shows the cache with "~".

## Architecture (parts that span several files)
- **lib + bin split:** all logic lives in the crate under `src/lib.rs`; `main.rs` only does terminal setup,
  panic safety and the frame loop. Tests build `noble::app::App` directly and draw it with `TestBackend` —
  keep new state/screens reachable from tests.
- **Event flow:** input, sensors, project scan, git, the AI collector and a reader/waiter thread per shell send
  `event::AppEvent` over a single `mpsc` channel; `App` (`app/mod.rs`) handles them in the main loop.
  New background job = new `AppEvent` variant + a matching branch in `App`.
- **Redraw (battery-friendly):** there is no fixed frame rate. `App::handle` returns `true` if the event changed
  something visible (e.g. a sensor event while in a terminal returns `false`); anything that changes over time
  (clock, animation, spinner, toast timeout, cursor blink) must be added to `App::redraw_after`, otherwise it
  freezes on screen. Non-event changes are reported via `dirty`/`take_dirty`.
  ≤60 fps under output; an idle terminal redraws once a minute, Home once a second.
- **Background work follows what is visible:** sensors via `SensorMode` (System: 1 s including the process
  list, Home: 1 s — 2 s on battery, otherwise 5 s), git status when a command finishes / Home opens / for a
  changed repo, AI quota only while Home is open (`AiReq::Visible`, immediately on entering). On battery
  (`App::on_battery`) the Home clock has no seconds and does not blink.
  When adding periodic work, run it only while the relevant screen is open; measure with
  `cargo test --release --test e2e idle -- --ignored --nocapture` (CPU ms per minute).
- **Mouse/hit-test:** `ui/*` drawing functions fill `hits: Vec<(Rect, Hit)>` while drawing; `app/input.rs`
  resolves clicks from that list (most recently added first). New clickable element = `app::Hit` variant +
  `hits.push` in drawing + handling in `input.rs`. Hover highlighting is based on the same list.
- **Terminal:** `term/layout.rs` pure split tree (knows nothing about PTYs), `term/pane.rs` PTY + `vt100` +
  shell integration (cwd tracking via OSC 7 / OSC 9;9), `term/input.rs` xterm key/mouse encoding and AltGr
  handling.
- **Prompt signal:** `term/pane.rs` `Callbacks` sets the `prompt` flag on every prompt (OSC 7 / 9;9 / 133);
  `App::on_pty_output` treats it as "command finished" → that repo's git status is refreshed with
  `ProjectReq::Refresh`, and a long command in a background tab raises a notification (`notify`).
  OSC 9 text / OSC 777 and the bell put an `alert` (◆) on the tab the same way.
- **Search/links:** `app/search.rs` (scrollback search bar, ctrl+click), `term/link.rs` (URL and `file:line`
  detection). Matches store absolute line numbers (0 = oldest scrollback line).
- **Terminal colors:** `theme.rs` `TermScheme`/`TermPalette`; `wt.rs` reads the PowerShell profile's scheme
  and user schemes from Windows Terminal's `settings.json` (JSONC) → `App::term_schemes`. Pane drawing
  (`ui/terminal.rs`) uses `TermPalette::resolve` every frame.
- **Claude hooks:** `hooks.rs` — when enabled in Settings, adds `noble hook <event>` to
  `~/.claude/settings.json` (writes a backup, removes only its own entries). `main.rs` handles this subcommand
  without opening the terminal; state is written to `data/agents/<NOBLE_INSTANCE>-<NOBLE_PANE>.json` files,
  which `App::tick` reads once a second (`apply_hook_records` → `AgentState`). Never touch the real file in
  tests.
- **cmd.exe commands:** the command is passed through the `NOBLE_LAUNCH` environment variable, not as an
  argument (`cmd /K %NOBLE_LAUNCH%`); portable-pty's `\"` escaping breaks quoted paths in cmd.
- **Persistence:** `config.rs` live-reloaded `config.toml` (error = toast, never a crash); `store.rs` stores
  recent dirs, the session, workspaces, AI usage history and UI state (`state.json`: welcome seen, pinned
  projects) with atomic JSON writes.

## Extension points
- New action: add it to `Action` in `src/keys.rs` (+ `ALL`, `id`, `title`, `group`), handle it in `App::run`,
  optionally bind a default key.
- New theme: append to `THEMES` in `src/theme.rs` — it appears in Settings automatically.
- New AI provider: `detect`/`fetch` + `ProviderDef` in `src/ai/providers.rs`, plus a payload test.
  Tokens only go to their own provider; they are never displayed, logged or refreshed.
