# NOBLE

**A retro-futurist HUD terminal workspace.** Real shells in tabs and splits, your git projects one keystroke away, live system sensors, and the remaining quota of your AI coding subscriptions — all in one cockpit that runs inside any modern terminal.

Single native binary (Rust + ratatui), ~22 MB RAM, and battery-friendly: an idle terminal tab costs about 0.05% of one core (it only redraws when something visible changes), the Home screen about 0.4%. On battery the Home clock drops its seconds and blinking colon and the summary sensors sample every 2 s.

```
 NOBLE   Home  System  │ +                                                                 ⚙ Settings   22:22

  ╺━┓ ╺━┓ ▄ ╺━┓ ╺━┓       Good evening, Mert
  ┏━┛ ┏━┛   ┏━┛ ┏━┛       Tuesday, 22 September
  ┗━╸ ┗━╸ ▀ ┗━╸ ┗━╸ 20    no terminals open

 ╭ Projects ────────────────────────────────────────────────────── 6 ─╮  ╭ AI usage ─────────────── 2m ago ─╮
 │▌ noble-rs                              main          ● 3       3m  │  │ Claude Code                  Max │
 │  api-server                            feature/auth… ✓         1h  │  │ 5h   ━━━━━━━━━━╸━━━━  71%  2h14m │
 │  dotfiles                              main          ● 1       1d  │  │ week ━━━━━━━━━━━━━━━  28%   4d2h │
 │  blog                                  master                  9d  │  │                                  │
 │  Noble                                 main          ● 54      5h  │  │ Codex                       Plus │
 │  scratch                               dev                     5w  │  │ 5h   ━━╸━━━━━━━━━━━━ ~18%  3h14m │
 │                                                                    │  │ week ━━━━━━━━━━━━━╸━ ~92%   1d2h │
 │                                                                    │  │ offline · showing data from 1h a │
 │                                                                    │  │                                  │
 │                                                                    │  ╰──────────────────────────────────╯
 │                                                                    │  ╭ System ────────────── details › ─╮
 │                                                                    │  │ CPU  ━━━━━━━━━━━━━━━━━━━━━━  65% │
 │                                                                    │  │       ⡀                        ⢀ │
 │                                                                    │  │ ⣶⣶⣶⣶⣶⣶⣷⣶⣤⣤⣄⣀⣀⣀⣰⣀⣀⣀⣀⣀⣀⣀⣤⣧⣴⣶⣶⣶⣶⣶⣶⣾ │
 │                                                                    │  │                                  │
 │                                                                    │  │ RAM  ━━━━━━━━━━━━━━━━━━━━━━  37% │
 │                                                                    │  │      11.8G of 32.0G              │
 │                                                                    │  │                                  │
 │  feat: HUD bridge · 2h                                             │  │ C:\  ━━━━━━━━━━━━━━━━━━━━━━  61% │
 │                                                                    │  │ D:\  ━━━━━━━━━━━━━━━━━━━━━━  88% │
 │  ⏎ Open   c Claude   x Codex   o Folder                   / search │  │                                  │
 ╰────────────────────────────────────────────────────────────────────╯  ╰──────────────────────────────────╯
 ⏎ open   c claude   / search   t terminal   s settings   ? help                               alt+p commands
```

## What it does

| Screen | |
|---|---|
| **Home** | A big clock and greeting, your git **projects** (branch, status — `● 3 changed`, `✓ clean`, `↑` commits to push, `↓` to pull — and last activity, refreshed as soon as a command finishes in that repo; the selected project's status is spelled out under the list), **AI usage** for the CLIs you are signed in to with a 24-hour usage graph, and a compact **system** summary. Projects with an open Claude/Codex session show `●` (running) or `◆` (waiting for you). Click a project twice (or press `⏎`) to open a terminal in it; the buttons under the list launch `claude` / `codex` there or open the folder. |
| **Terminals** | Real shells (ConPTY on Windows) in tabs. Each pane has buttons to split right `┃`, split down `━`, zoom `⤢` and close `✕`; drag any border to resize. Drag to select text (copied), right-click to paste, wheel to scroll, `prefix /` to **search the scrollback**, `ctrl+click` to open **links and `file:line` paths**. A background tab gets a `◆` when a long command finishes, the bell rings or an app sends a notification (OSC 9 / 777). Full-screen apps like vim, htop and Claude Code work, including their mouse support. |
| **System** | CPU history and per-core load, memory, network, disks, **battery** (laptops: charge, time left or time to full — from the OS, or estimated from the charge rate when the OS has none; warns at 20% and 10%) and a sortable process table with a confirmed "end task". |
| **Settings** | Pick one of **20 themes** (each card previews its own colours), give terminal panes their own **color scheme** — by default the one your PowerShell profile uses in Windows Terminal (read from its `settings.json`, including your custom schemes), or any of Windows Terminal's built-in schemes (Campbell, Campbell Powershell, One Half, Solarized, Tango, Dark+, Vintage, CGA) plus Light Gray, Graphite and Paper — picked from a live-preview list, toggle display options, choose the shell and prefix key, and turn AI providers on or off. Every change is saved to `config.toml` instantly. |

The first launch shows a short welcome card: what NOBLE found, the four keys worth knowing and a choice of prefix key (so it doesn't fight your shell). Everything is clickable and lights up under the mouse; right-click a tab, a pane's title bar or a project for a context menu, drag tabs to reorder them, double-click one to rename it, and hover a project for quick `code` / `pull` / pin buttons (pinned projects stay on top); pages slide in from the side and zoomed panes grow out of (and shrink back into) their place — turn animations off in Settings if you prefer. The keyboard works too (`?` shows every shortcut). On quit your tabs, splits and each shell's folder are saved and restored next time.

## Install

Ready-made binaries for Windows and Linux are attached to each GitHub release. To build from source you need a Rust toolchain (1.88+, edition 2024):

```sh
git clone <this repo> noble && cd noble
cargo install --path .        # puts `noble` on your PATH (~/.cargo/bin)
noble
```

On Windows, reinstalling while NOBLE is open fails with "access denied" because the running `noble.exe` is locked; `install.cmd` renames it out of the way first, so use that instead of `cargo install` when updating.

Or build without installing: `cargo build --release` → `target/release/noble(.exe)`.

**Terminal:** any truecolor terminal with a regular monospace font works (no Nerd Font needed) — Windows Terminal, WezTerm, Kitty, Alacritty, iTerm2. Cascadia Code / Cascadia Mono render every glyph NOBLE uses (box drawing, block elements, braille).

**Windows Terminal profile** (optional) — open NOBLE in its own tab from the dropdown:

```json
{ "name": "NOBLE", "commandline": "noble.exe", "icon": "⌂", "font": { "face": "Cascadia Mono" } }
```

```
noble [--config <path>] [--no-boot]
      --paths      print config and data locations
      --version    print version
```

## Keys

NOBLE uses a tmux-style **prefix** (default `ctrl+a`, press it twice to send a literal `ctrl+a` to the shell) plus a few direct shortcuts. Everything is rebindable, and `?` shows the live reference.

| Direct | | Prefix, then | |
|---|---|---|---|
| `alt+1…9` | go to tab | `t` `c` | new tab |
| `alt+0` | bridge | `v` `\|` | split right |
| `alt+t` | new tab | `s` `-` | split down |
| `alt+p` | command palette | `x` / `X` | close pane / tab |
| `alt+m` | system monitor | `z` | zoom pane |
| `alt+s` | settings | `S` | settings |
| `alt+z` | zoom pane | `← → ↑ ↓` `o` | move focus |
| `alt+o` | next pane | `shift+arrows` `H J K L` | move divider |
| `alt+.` / `alt+,` | next / previous tab | | |
| `shift+pgup/pgdn` | scrollback | `n` `p` `1…9` `0` | tabs · bridge |
| | | `/` `f` | search scrollback |
| | | `,` `w` `:` `?` `r` `q` | rename · save workspace · palette · help · reload config · quit |

**Home:** `↑↓` select project · `⏎` open terminal · `c` Claude · `x` Codex · `/` search · `t` terminal at home · `o` open folder · `w` save workspace · `a` add a project folder · `r` rescan / `R` refresh AI · `m` system · `s` settings · `q` quit.

**Settings:** `↑↓←→` move · `⏎`/space change · `←→` also cycles values · `esc` back.

**System:** `↑↓` select · `c m p n` sort by CPU / memory / pid / name (again to flip) · `/` filter · `K` or `del` terminate (asks first) · `esc` back.

**Search:** type to find (case-insensitive) · `⏎`/`↑` older match · `↓`/`shift+⏎` newer · `esc` close and jump back to the bottom.

**Mouse:** `ctrl+click` opens a URL (including OSC 8 hyperlinks printed by tools like `ls --hyperlink` or compilers) in the browser or a `path:line:col` in VS Code (`code -g`, otherwise the default app) · right-click a tab, pane title or project for a menu · drag tabs to reorder · click tabs, panes, rows and chips · drag dividers · drag to select text (copied on release) · right-click pastes · wheel scrolls history and lists · `shift+drag` selects even inside apps that capture the mouse.

AltGr symbols (`@ { } [ ] \ | ~ €` on Turkish, German, Polish… layouts) are passed through as characters, not as `ctrl+alt` chords.

## Configuration

`noble --paths` prints the locations (Windows: `%APPDATA%\noble\config.toml`, data in `%LOCALAPPDATA%\noble`; `NOBLE_HOME` moves both). The file is created with comments on first launch and **reloaded live** when saved; errors show as a toast and never crash the app.

```toml
[general]
theme = "amber"          # any of the 20 themes in Settings
transparent = false      # let the terminal's own background (blur/opacity) show
boot_animation = true
clock_24h = true
show_seconds = true
operator = ""            # name in the greeting; empty = your user name

[terminal]
shell = ""               # empty = pwsh → powershell → cmd on Windows, $SHELL elsewhere
shell_args = []
scrollback = 5000
restore_session = true
copy_on_select = true
colors = "windows-terminal"  # pane colors: windows-terminal (your PowerShell scheme) | theme | campbell | dark-plus | light-gray | "wt:<your scheme>" …
background = ""          # override the scheme's background, e.g. "#c8c8c8"
foreground = ""          # override the scheme's text color
notify = true            # toast + bell when a background tab needs attention
notify_after = 10        # report background commands that ran at least this many seconds (0 = off)

[keys]
prefix = "ctrl+a"
[keys.prefix_bindings]   # key after the prefix → action ("none" unbinds)
"%" = "split_right"
[keys.direct_bindings]   # global chords
"alt+v" = "split_right"

[projects]
roots = []               # empty = Desktop, Documents, source/repos, projects, code, dev, src, repos …
max_depth = 4
exclude = []

[ai]
enabled = true
refresh_minutes = 5
providers = ["claude", "codex"]
warn_at = 90             # warn once when a quota window reaches this percent (0 = off)

[[launchers]]
key = "c"
name = "claude"
command = "claude"
```

**Actions** for bindings: `bridge system new_tab close_tab next_tab prev_tab tab_1…tab_9 split_right split_down close_pane zoom focus_left focus_right focus_up focus_down focus_next resize_left resize_right resize_up resize_down palette help quit reload_config open_config cycle_theme refresh_ai rescan_projects rename_tab save_workspace scroll_up scroll_down search send_prefix`.

## AI quota sources

Usage is read from the logins that the official CLIs already keep on your machine. Tokens are sent only to their own provider, never displayed or logged, and never refreshed by NOBLE (so it cannot race the CLI's own token rotation). The last good numbers are cached and shown with `~` until the next successful fetch.

| Provider | Shown | Source |
|---|---|---|
| Claude Code | 5-hour and weekly limits, plan | `~/.claude/.credentials.json` → Anthropic OAuth usage endpoint |
| Codex / ChatGPT | 5-hour and weekly limits, plan | `~/.codex/auth.json` + `codex app-server` (`account/rateLimits/read`) |

Only providers you are signed in to are shown. Quotas are fetched only while the Home screen is open — right away when you come back to it (unless the data is under 30 s old) and every `refresh_minutes` while it stays open — so NOBLE makes no network calls and starts no `codex` process while you work in a terminal. Each successful fetch is also appended to `ai-history.json` in the data folder (kept for 8 days) to draw the 24-hour graph and to warn when, at the current pace, a window would fill up before it resets.

### Claude Code status (optional)

Without help NOBLE can only guess whether a Claude session is busy. Turn on **Settings → AI usage → Claude Code status hooks** and NOBLE adds a few hooks to `~/.claude/settings.json` (a backup is written next to it; turning the setting off removes exactly those entries). Claude then runs `noble hook <event>` on prompt / stop / notification / session start / session end; the command writes one small file per pane into NOBLE's data folder and exits — outside NOBLE it does nothing. The Home screen then lists every Claude session as **working**, **needs you** or **your turn**, and a background tab lights up the moment Claude asks for permission.

## Shell integration

NOBLE learns each pane's working directory from OSC 7 / OSC 9;9. For PowerShell it wraps your existing prompt (oh-my-posh included) to emit OSC 9;9; for `cmd.exe` it sets a `PROMPT` that does the same unless you already have one. Bash/zsh users whose prompt emits OSC 7 get it automatically. This is what lets splits open in the current directory and sessions restore to where you left off. Each prompt (OSC 7, OSC 9;9 or OSC 133 marks) also tells NOBLE that the previous command finished: it refreshes that repository's git status and, for background tabs, reports long-running commands.

## Architecture

```
src/
  main.rs            terminal setup, panic safety, event-driven frame loop (≤60 fps under output, redraws only when something visible changes)
  app/               state + input: mod.rs (events, notifications, agent state, config reload), input.rs (keys, mouse),
                     menu.rs (context menus, tab/project actions),
                     ops.rs (tabs, panes, launchers, sessions), search.rs (scrollback search, links),
                     palette.rs, settings.rs
  term/              layout.rs (pure split tree), pane.rs (PTY + vt100 + shell integration),
                     link.rs (URL / file:line detection), input.rs (xterm key/mouse encoding, AltGr)
  ui/                hud.rs (bounds-safe primitives: frames, bars, buttons, braille graphs, big digits),
                     bridge.rs (Home), terminal.rs, system.rs, settings.rs, overlay.rs, boot.rs
  ai/                providers.rs (Claude, Codex), json.rs (tolerant window parsing), collector
  sensors.rs         sysinfo sampler thread
  projects.rs        repository scan + git status workers
  store.rs           recent dirs (frecency), session, workspaces, AI usage history, UI state (atomic JSON writes)
  hooks.rs           Claude Code hook install/remove and the `noble hook` state files
  wt.rs battery.rs   Windows Terminal color schemes · battery readers (Windows, Linux, macOS)
  config.rs keys.rs theme.rs util.rs event.rs
```

Background threads (input, sensors, project scan, AI collector, one reader + waiter per shell) talk to the main loop through a single channel; only the main thread touches UI state. PTY output marks a pane dirty and the loop coalesces redraws. A panic in a background thread is written to `noble.log` instead of tearing down the screen.

## Development

```sh
cargo test                     # unit + headless render + real-PTY + end-to-end tests
cargo clippy --all-targets     # kept warning-free
cargo fmt
cargo test --release --test e2e idle -- --ignored --nocapture   # idle CPU/RAM check
```

- `tests/render.rs` draws every screen at seven terminal sizes (160×45 down to 30×8) with a headless backend, asserts nothing panics or overflows, and writes text snapshots to `target/audit/` — open them to review layout changes without launching the app. It also drives real shells: output, splits, zoom, `cd` tracking (PowerShell and cmd), launchers.
- `tests/e2e.rs` runs the compiled binary inside a pseudo-terminal, types into it and reads the screen back: open tab → run command → split → palette → quit → session restored on relaunch.
- New AI provider: add `detect`/`fetch` functions and a `ProviderDef` in `src/ai/providers.rs`, plus a payload test.
- New action: add it to `Action` (+ `ALL`, `id`, `title`, `group`) in `src/keys.rs`, handle it in `App::run`, optionally bind a default key.
- New theme: append to `THEMES` in `src/theme.rs` — it appears in Settings automatically.
- After every change: `install.cmd` (Windows) or `cargo install --path .` so `noble` runs the latest build.
- CI (`.github/workflows/ci.yml`) runs fmt, clippy and the full test suite on Windows, and clippy + unit tests on Linux; pushing a `v*` tag builds release binaries (`release.yml`).

## License

MIT
