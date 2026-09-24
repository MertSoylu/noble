# Contributing to NOBLE

Thanks for taking the time to contribute! Bug reports, ideas, documentation fixes and pull requests are all
welcome. This guide covers how to get set up, how the code is organised and what a good pull request looks like.

- [Ways to contribute](#ways-to-contribute)
- [Development setup](#development-setup)
- [Tests](#tests)
- [Architecture](#architecture)
- [Common changes](#common-changes)
- [Pull request checklist](#pull-request-checklist)

## Ways to contribute

- **Report a bug:** open an [issue](https://github.com/MertSoylu/noble/issues/new/choose) with your OS,
  terminal, shell and the output of `noble --version`. A screenshot helps a lot for layout problems.
- **Suggest a feature:** open a feature request and describe the problem you want solved before the solution.
  NOBLE aims to stay simple and uncluttered, so not every idea will fit, but every one is read.
- **Send a pull request:** for anything larger than a small fix, please open an issue first so we can agree on
  the approach.
- **Security issues:** please do **not** open a public issue. See [SECURITY.md](SECURITY.md).

## Development setup

You need a Rust toolchain, **1.88 or newer** (edition 2024).

```sh
git clone https://github.com/MertSoylu/noble && cd noble
cargo run -- --no-boot          # run the working tree, skipping the boot animation
```

Useful flags and variables:

| | |
|---|---|
| `--no-boot` | skip the boot animation |
| `--paths` | print where the config and data files live |
| `--config <path>` | use another config file |
| `NOBLE_HOME=<dir>` | move config **and** data into one folder, so you can experiment without touching your real setup |

### Using a stable build and a dev build side by side

If you use NOBLE every day and also hack on it, install the working tree under a **different name**:

```sh
install.cmd                     # Windows: builds --release and installs it as `noble-dev`
./install.sh                    # Linux / macOS: the same
```

`noble` (from a release or `cargo install noble`) stays untouched, and `noble-dev` runs your latest changes.
The dev build shows **NOBLE dev** in its top bar and window title and saves its open tabs to a separate
session file, so the two never overwrite each other's tabs. Both share the same config and data (projects,
pins, AI cache, Claude hook status). For a completely separate sandbox add `NOBLE_HOME`:

```powershell
$env:NOBLE_HOME = "$env:TEMP\noble-sandbox"; noble-dev
```

Both scripts work while `noble-dev` is running: Windows refuses to overwrite a running `.exe` but allows
renaming it, so `install.cmd` moves the old binary out of the way first; `install.sh` puts the new file in
place with a rename, which Unix allows for a running binary.

## Tests

```sh
cargo test                        # everything: unit, headless render, real PTY and end-to-end tests
cargo test --lib <name>           # unit tests in src/
cargo test --test render          # render + PTY tests; add a test name to run one
cargo test --test e2e             # runs the compiled binary inside a pseudo-terminal
cargo clippy --all-targets        # must stay warning-free (CI uses -D warnings)
cargo fmt                         # rustfmt.toml: max_width = 120
```

- **`tests/render.rs`** draws every screen with a headless backend at seven terminal sizes, from 160×45 down to
  30×8, asserts nothing panics or overflows and writes text snapshots to `target/audit/*.txt`. After a UI change,
  open those files to review the layout at every size without launching the app. It also drives real shells:
  output, splits, zoom, `cd` tracking (PowerShell and cmd) and launchers.
- **`tests/e2e.rs`** runs the real binary: open a tab, run a command, split, open the palette, quit and check
  the session is restored on the next launch.
- **Performance checks** (ignored by default, run in release mode):

  ```sh
  cargo test --release --test e2e idle -- --ignored --nocapture            # idle CPU / RAM per minute
  cargo test --release --test render heavy_output -- --ignored --nocapture # 50k lines of output
  ```

CI runs `fmt`, `clippy -D warnings` and the full suite on Windows, and `clippy` plus unit tests on Linux.

### Screenshots

The images in the README are generated, not captured by hand:

```sh
cargo run --example screenshots   # writes docs/assets/*.svg
```

The example builds the app headless with fake data (no real paths, names or accounts), renders each screen and
converts the frame to SVG. Box drawing, block and braille characters are drawn as vectors, so the images look
the same in every browser. Please regenerate them when a change affects a screen shown in the README.

## Architecture

All logic lives in the library crate (`src/lib.rs`). `main.rs` only sets up the terminal, installs the panic
handler and runs the frame loop, which lets the tests build `noble::app::App` directly and draw it with
ratatui's `TestBackend`.

```
src/
  main.rs            terminal setup, panic safety, event-driven frame loop
  app/               state and input: mod.rs (events, notifications, agent state, config reload),
                     input.rs (keys, mouse), menu.rs (context menus), ops.rs (tabs, panes, launchers,
                     sessions), search.rs (scrollback search, links), palette.rs, settings.rs
  term/              layout.rs (pure split tree), pane.rs (PTY + vt100 + shell integration),
                     link.rs (URL / file:line detection), input.rs (xterm key/mouse encoding, AltGr)
  ui/                hud.rs (bounds-safe drawing primitives), bridge.rs (Home), terminal.rs,
                     system.rs, settings.rs, overlay.rs, boot.rs
  ai/                providers.rs (one adapter per provider), json.rs (tolerant parsing), collector
  sensors.rs         sysinfo sampler thread
  projects.rs        repository scan and git status workers
  store.rs           recent dirs, session, workspaces, AI history, UI state (atomic JSON writes)
  hooks.rs           Claude Code hook install/remove and `noble hook` state files
  wt.rs battery.rs   Windows Terminal color schemes · battery readers
  config.rs keys.rs theme.rs util.rs event.rs
```

A few rules hold the design together:

- **One event channel.** Input, sensors, project scan, git, the AI collector and a reader/waiter pair per shell
  all send an `event::AppEvent` over a single `mpsc` channel. Only the main thread changes state. A new
  background job means a new `AppEvent` variant plus a matching branch in `App`.
- **Redraw only when needed.** There is no fixed frame rate. `App::handle` returns `true` when an event changed
  something visible. Anything that changes over time (clock, animation, spinner, toast timeout, cursor blink)
  must be reported by `App::redraw_after`, otherwise it freezes on screen.
- **Background work follows the screen.** Sensors, git refreshes and AI quota requests run only while the screen
  that shows them is open. Measure new periodic work with the idle test above.
- **Mouse via hit lists.** Drawing functions push `(Rect, Hit)` pairs while they draw. `app/input.rs` resolves
  clicks and hover from that list, newest first. A new clickable element means a `Hit` variant, a `hits.push`
  in the drawing code and handling in `input.rs`.
- **Bounds-safe drawing.** UI code draws only through the `ui/hud.rs` primitives, never by indexing the
  `Buffer` directly, so nothing can panic at small sizes.
- **Errors are values.** Network or CLI failures never panic: providers return `Err(String)` and the UI shows
  the cached value with `~`. Config errors become a toast.
- **Secrets stay put.** AI tokens are only sent to their own provider. They are never displayed, logged or
  refreshed by NOBLE.

## Common changes

- **New action:** add it to `Action` in `src/keys.rs` (plus `ALL`, `id`, `title`, `group`), handle it in
  `App::run`, and optionally bind a default key.
- **New theme:** append it to `THEMES` in `src/theme.rs`. It shows up in Settings automatically.
- **New AI provider:** add `detect` / `fetch` functions and a `ProviderDef` in `src/ai/providers.rs`, plus a test
  that parses a sample payload. Read credentials only from files the provider's own CLI already keeps.
- **New quick-launch default:** add it to `DEFAULT_LAUNCHERS` in `src/config.rs` (and the commented template).
  Older configs pick it up automatically.

## Pull request checklist

- [ ] `cargo fmt`, `cargo clippy --all-targets` and `cargo test` pass
- [ ] UI changes checked in `target/audit/*.txt`, including the small sizes
- [ ] README screenshots regenerated if a pictured screen changed
- [ ] User-facing changes noted under **Unreleased** in [CHANGELOG.md](CHANGELOG.md)
- [ ] Code comments, identifiers and UI text are in English

By contributing you agree that your contributions are licensed under the [MIT License](LICENSE).
