<p align="center">
  <img src="docs/assets/banner.svg" alt="NOBLE — a retro-futurist HUD terminal workspace" width="100%">
</p>

<p align="center">
  <a href="https://github.com/MertSoylu/noble/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/MertSoylu/noble/ci.yml?branch=main&style=flat-square&label=CI" alt="CI"></a>
  <a href="https://github.com/MertSoylu/noble/releases"><img src="https://img.shields.io/github/v/release/MertSoylu/noble?style=flat-square&color=ffb020" alt="Release"></a>
  <a href="https://crates.io/crates/noble"><img src="https://img.shields.io/crates/v/noble?style=flat-square&color=5fd7d0" alt="crates.io"></a>
  <img src="https://img.shields.io/badge/rust-1.95%2B-orange?style=flat-square&logo=rust" alt="Rust 1.95+">
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey?style=flat-square" alt="Platforms">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT License"></a>
  <a href="https://ratatui.rs/"><img src="https://ratatui.rs/built-with-ratatui/badge.svg" alt="Built With Ratatui" height="20"></a>
</p>

<p align="center">
  <b>Real shells in tabs and splits, your git projects one keystroke away, live system sensors<br>
  and the remaining quota of your AI coding subscriptions, all in one cockpit that runs inside your terminal.</b>
</p>

<p align="center">
  <a href="https://noble.mertsoylu.dev"><b>Website</b></a> ·
  <a href="https://noble.mertsoylu.dev/docs">Docs</a> ·
  <a href="#-install">Install</a> ·
  <a href="#-features">Features</a> ·
  <a href="#-screens">Screens</a> ·
  <a href="#-keys">Keys</a> ·
  <a href="#-configuration">Configuration</a> ·
  <a href="#-ai-agents">AI agents</a> ·
  <a href="#-ai-quota">AI quota</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <img src="docs/assets/home.svg" alt="NOBLE Home screen: projects, recent commits, AI usage and system sensors" width="100%">
</p>

NOBLE is a single native binary (Rust + [ratatui](https://ratatui.rs)) that turns your terminal into a
workspace: open a project and its shell with one key, start Claude Code or Codex right there, and keep
an eye on your git status, your CPU and how much of your 5-hour AI quota is left, without leaving the keyboard, or
the mouse, since everything is clickable.

It is also **light**: about 22 MB of RAM, and it only redraws when something visible changes. An idle terminal
tab costs about 0.05% of one core and the Home screen about 0.4%.

## ✨ Features

<table>
<tr>
<td width="50%" valign="top">

**🖥 Real terminals**<br>
ConPTY on Windows, a PTY on Linux and macOS. Tabs, splits, zoom, drag to resize, scrollback search, `ctrl+click` on URLs
and `file:line` paths. vim, htop and Claude Code run full-screen with mouse support. Each pane's title shows
how the last command ended (`✓ 2.4s`, `✗ 1 · 12s`) and a live timer while one runs. Scrolled back, a pane shows a
position bar with the search matches on it and a `↓ live · 12` chip counting the new lines below. The focused
or hovered pane shows its buttons on the title line: `◫` split right, `⊟` split down, `⤢` zoom, `✕` close.
In a split tab the panes without the focus fade a little (`dim_inactive`), so the focused one stands out.
Focus mode (`prefix f`, or `focus_mode` in the config) hides the top and bottom bars on a terminal tab, so its panes
get the whole screen: the bars come back while the prefix is armed or the pointer rests on the top or bottom row, and a
`◆ 2` chip on the bottom row shows the markers of the other tabs.
Programs can rely on the usual terminal queries: synchronized output (no half-drawn frames), the pane's colors
(OSC 4 / 10 / 11 / 12, `COLORFGBG`, color scheme notices) so AI CLIs pick the right light or dark theme, focus
reports, XTVERSION, DECRQM, the kitty keyboard protocol (flag 1 only, one flag stack per screen) and, on Windows,
ConPTY's win32-input-mode.

</td>
<td width="50%" valign="top">

**📁 Projects at a glance**<br>
Finds your git repositories and shows branch, changes, commits to push or pull, recent commits and changed
files. The status refreshes as soon as a command finishes in that repository. Add any folder by hand (`A`) or
remove one you do not want (⋯ → Remove from list). The selected project's card also shows its branch's pull
request and CI state (`PR #42 open · ✓ CI 12/12`, with the [GitHub CLI](https://cli.github.com) signed in via
`gh auth login`) and the ports that processes running inside the project listen on (`⇄ :3000 :5173`), on
Windows, Linux and macOS; click either one to open it. The card also lists the project's quick tasks as a
TASKS row of chips (`▸ npm run dev`, `▸ cargo test`): the `package.json` scripts (npm, pnpm, yarn or bun by its
lockfile), the Cargo build, test and run commands, and the plain Makefile targets and justfile recipes. Click a
chip, or press `T` (shift+t) for the list, to run it in a new tab in the project folder.

</td>
</tr>
<tr>
<td valign="top">

**🤖 AI quota and sessions**<br>
5-hour and weekly limits for Claude Code, Codex, Antigravity, OpenCode Go, Kilo Code and Command Code, read
from the logins the CLIs already keep. See which agent is working and which one is waiting for you (every agent with status hooks on),
on Home, in each pane's title and as a dot on its tab; prefix `a` jumps to the next agent waiting for you.

</td>
<td valign="top">

**🚀 Quick launch**<br>
Start Claude Code, Codex, OpenCode, Copilot, Cursor, Cline and more in the selected project with a single
key. Only the CLIs installed on your PATH are shown.

</td>
</tr>
<tr>
<td valign="top">

**📊 System monitor**<br>
CPU history and per-core load, memory, network, disks, battery (with a time estimate when the OS has none)
and a sortable process table.

</td>
<td valign="top">

**🎨 22 themes, live config**<br>
Amber, Ice, Synthwave, Catppuccin, Tokyo Night, Nord, Gruvbox… Terminal panes follow the theme, a built-in
scheme or, on Windows, your Windows Terminal scheme. `config.toml` reloads live and a bad value never crashes
the app.

</td>
</tr>
<tr>
<td valign="top">

**🖱 Mouse first, keyboard too**<br>
Every button, tab, row and chip is clickable, with context menus on right-click. A tmux-style prefix key and
a command palette (`alt+p`, prefix `:` in a terminal) cover the keyboard side.

</td>
<td valign="top">

**💾 Sessions and workspaces**<br>
Tabs, splits and each shell's folder are restored on the next launch; a folder that is gone or does not answer
(a dead network share) opens in home instead. With several NOBLE windows open, each one saves its own tabs and
the next launch brings back all of them; a window opened while another is running starts empty. Quick-launch
tabs run their command (e.g. `claude`) again. An agent pane (Claude Code, Codex, GitHub Copilot, OpenCode, Antigravity
CLI) picks its last conversation up again on the same path: `claude --resume`, `codex resume`,
`copilot --resume=<id>`, `opencode --session`, `agy --conversation`, added to its own launcher
(Settings → Resume agent conversations; `resume_agents`, on by default). Saved workspaces and
reopened tabs (prefix `T`) do the same. The session is saved a couple of seconds after every change and
when the window closes, the process is terminated or the OS logs off. Save named workspaces, reopen or delete
them from the palette. `noble .` (or `noble <folder>`) opens a shell tab in that folder.

</td>
</tr>
</table>

## 📸 Screens

<p align="center">
  <img src="docs/assets/terminals.svg" alt="Claude Code and Codex running side by side in split panes" width="100%">
</p>

<table>
<tr>
<td width="50%"><img src="docs/assets/system.svg" alt="System monitor"><p align="center"><sub><b>System:</b> CPU, cores, memory, network, disks, battery, processes</sub></p></td>
<td width="50%"><img src="docs/assets/palette.svg" alt="Command palette"><p align="center"><sub><b>Command palette:</b> every action, project, tab and theme</sub></p></td>
</tr>
<tr>
<td width="50%"><img src="docs/assets/settings.svg" alt="Settings"><p align="center"><sub><b>Settings:</b> saved to <code>config.toml</code> instantly</sub></p></td>
<td width="50%"><img src="docs/assets/themes.svg" alt="Six of the twenty-two themes"><p align="center"><sub><b>Themes:</b> Ice, Synthwave, Catppuccin, Gruvbox, Nord, Latte</sub></p></td>
</tr>
</table>

## 📦 Install

**Prebuilt binaries:** download the archive for Windows (x86_64, ARM64), Linux (x86_64, ARM64) or macOS (Apple
Silicon, Intel) from the [latest release](https://github.com/MertSoylu/noble/releases/latest), unpack it and put
`noble` on your PATH. The Linux binaries are static, so they run on any distribution; the Windows ones need no
VC++ redistributable. The macOS binaries are
not notarized: a file downloaded with a browser needs `xattr -d com.apple.quarantine noble` once (not needed
with `curl` or `noble update`). Each archive has a `.sha256` file next to it: check it with
`sha256sum -c noble-linux-x86_64.tar.gz.sha256` on Linux, `shasum -a 256 -c noble-macos-aarch64.tar.gz.sha256`
on macOS, or compare it with `Get-FileHash noble-windows-x86_64.zip` in PowerShell on Windows.

**winget (Windows):** `winget install MertSoylu.Noble` is planned but not available yet: NOBLE is not in the winget
community repository (microsoft/winget-pkgs) yet. Until then use the prebuilt archive above, Cargo or the source install
below.

**With Cargo** (Rust 1.95+):

```sh
cargo install noble
```

Or skip the compile with [cargo-binstall](https://github.com/cargo-bins/cargo-binstall), which downloads the same
prebuilt archive for Windows, Linux or macOS: `cargo binstall noble`.

**From source:**

```sh
git clone https://github.com/MertSoylu/noble && cd noble
cargo install --path .
```

Then run `noble`. On the first launch a short welcome card shows what NOBLE found and the four keys worth
knowing, and lets you set up the basics in a few keystrokes: the theme and terminal colors (previewed live as
you cycle them with ←→), the shell when more than one is installed (PowerShell, cmd, Git Bash on Windows; bash,
zsh, fish … on Linux and macOS) and a prefix key that doesn't clash with your shell. ⏎ keeps your choices, esc
keeps the defaults; everything can be changed later in Settings.

**Updating:** NOBLE checks GitHub for a new release once a day and shows it at the bottom right. Click
**update** there, or run `noble update` (`noble update --check` only reports). It downloads the prebuilt
binary for your platform, checks it against the release's SHA-256 checksum (a mismatch or a missing checksum
stops the update and leaves the installed binary alone), refuses any address that is not GitHub over https and any
archive that holds links or paths outside its folder, and replaces the installed one; restart NOBLE afterwards. Turn the check off with
`check_updates = false` or in Settings.

<details>
<summary><b>Terminal requirements and a Windows Terminal profile</b></summary>

<br>

Any truecolor terminal with a regular monospace font works, no Nerd Font needed: Windows Terminal, WezTerm,
Kitty, Alacritty, GNOME Terminal, Konsole, iTerm2, Ghostty and Terminal.app. Terminal.app before macOS 26
has no truecolor: NOBLE then draws with the nearest colors of the 256-color palette. Cascadia Code / Cascadia Mono render every glyph NOBLE uses
(box drawing, block elements, braille).

Copy and paste use the system clipboard (Windows, macOS, X11 and Wayland). Without one, e.g. over SSH, copied text is
sent to your terminal with OSC 52, links are copied instead of opened, and files and the config open in a
terminal editor (`$EDITOR`, else nano or vim). A program running in a pane can also set the clipboard with OSC 52;
`osc52_write = false` under `[terminal]` (Settings → "Clipboard from programs") ignores those requests.

To open NOBLE in its own tab from the Windows Terminal dropdown:

```json
{ "name": "NOBLE", "commandline": "noble.exe", "icon": "⌂", "font": { "face": "Cascadia Mono" } }
```

```
noble [--config <path>] [--no-boot] [<dir>]   open NOBLE; <dir> opens a shell tab in that folder
noble update [--check]                        download and install the latest release (--check only reports)
noble doctor [--config <path>]                print a setup report for bug reports (never opens the terminal)
      --paths      print config and data locations
      --version    print version
      -h, --help   print the help
```

</details>

> [!NOTE]
> Windows, Linux and macOS are all supported: CI runs the whole test suite, real terminals and end-to-end
> included, on each of them. On macOS, turn on **Option as Meta** in your terminal for the `alt+` shortcuts
> (see Troubleshooting); the prefix works either way.

## ⌨ Keys

NOBLE uses a tmux-style **prefix** (default `ctrl+a`; press it twice to send a literal `ctrl+a` to the shell)
plus a few direct shortcuts. Everything can be rebound: **Settings → Key bindings** lists every action with its prefix and
direct keys (↑↓ select, `/` filter, `⏎` rebinds the prefix key, `d` the direct chord, `x` unbinds, `r` restores the default;
a key another action has is asked about first), and each change is saved to `config.toml` at once. `?` shows the live
reference: type to filter it, ↑↓ pick a row and ⏎ (or a click) runs it.

The direct shortcuts marked \* are shell keys too (last argument, transpose words, fish's sudo and pager …), so in
a terminal they go to the shell and work everywhere else; use the prefix column there instead. `shell_first =
false` (Settings → Leave shell keys to the shell) gives them back to NOBLE everywhere. The first time each such
key goes to the shell, the status bar says once which chord NOBLE uses for that action (e.g. `alt+t went to the
shell · NOBLE: ctrl+a t`).

When an app in a pane needs one of NOBLE's other shortcuts (e.g. `alt+m`, `alt+z`), **prefix `i`** locks the
pane's keys: every shortcut goes to the app (🔒 KEYS on the pane) until prefix `i` again; the prefix itself keeps
working. With `passthrough = "once"` (Settings → Pass keys to the app) there is no lock: prefix + a shortcut
(e.g. `ctrl+a alt+m`) sends just that key, and prefix `i` sends the next key.

| Direct | | Prefix, then | |
|---|---|---|---|
| `alt+1…9` | go to tab | `t` `c` | new tab |
| `alt+0` | Home | `v` `\|` | split right |
| `alt+t`\* | new tab | `s` `-` | split down |
| `alt+p`\* | command palette | `x` / `X` | close pane / tab |
| | | `T` | reopen the last closed tab |
| `alt+m` | system monitor | `z` | zoom pane |
| | | `=` `}` `!` | equalize splits · swap with the next pane · move the pane to a new tab |
| `alt+s`\* | settings | `S` `m` | settings · system monitor |
| `alt+z` | zoom pane | `← → ↑ ↓` `o` | move focus |
| `alt+o` | next pane | `shift+arrows` `H J K L` | move divider |
| `alt+.` / `alt+,`\* | next / previous tab | `n` `p` `1…9` `0` `b` | tabs · Home |
| `shift+pgup/pgdn` | scrollback | `/` | search scrollback |
| | | `f` | focus mode: hide the top and bottom bars on a terminal tab |
| | | `pgup` `pgdn` `[` `]` | scroll up / down (page) |
| | | `<` `>` `.` | move tab left / right · pane menu (copy path, open folder …) |
| | | `i` | pass keys to the app (lock / next key) |
| | | `a` | jump to the next agent waiting for you (needs you first, then your turn, then error) |
| | | `u` | list the links on the pane's screen (URLs, `file:line`) and open one |
| | | `y` | send the last command's output to the agent pane (pasted, not run: press ⏎ yourself) |
| | | `,` `w` `:` `space` `?` `r` `q` | rename · save workspace · palette · keys · reload config · quit |

A shell with NOBLE's hooks (PowerShell, bash 4.4+, zsh, fish) marks where each command's output starts and ends, so a
terminal tab can jump between commands (pane menu, palette: Jump to Previous / Next Command), copy the last output, or
send it to the agent in the same tab (or else the one last sent to) with its context line (`~/app $ cargo test → exit 101`). A failed command's ✗
badge scrolls to its output when clicked. cmd.exe sends no marks, and bash 3.2 no command-start mark (OSC 133;C),
so these actions do nothing there.

<details>
<summary><b>Per-screen keys and mouse</b></summary>

<br>

- **Home:** `↑↓` select project · `⏎` open terminal · `→` then `⏎` pin to top (★) or more actions (⋯) · `c` Claude · `x` Codex · other AI CLIs by their shortcut
  (Settings → Quick launch) · `/` search (or click its label) · `t` terminal at home · `o` open folder · `w` save workspace ·
  `a` add a project or a folder to scan (the prompt decides: a git repository or a folder without repositories is added as a
  project, a folder holding repositories is scanned; `tab` forces one of the modes) · ⋯ → Remove from list hides a project for good (`a` brings it back) ·
  `r` rescan / `R` refresh AI · `p` or `:` palette · `?` keys · `1…9` go to tab ·
  `m` system · `s` settings · `q` quit · `T` run a task of the selected project (only when it has tasks; the
  palette lists them; a launcher on `T` keeps the key). The selected project's card: click `PR #…` to open its pull request in the
  browser, or a port (`⇄ :3000`) to open http://localhost:<port>; click a task chip to run it. A `↻ claude · 2h ago`
  line shows when Claude Code has a conversation in that folder (the time its `~/.claude/projects` folder changed);
  click it to open a tab that runs `claude --continue` there. Claude Code background sessions (`claude --bg`) are listed
  in the AI usage panel under the agent sessions; a click attaches to one in a new tab (`claude attach <id>`), the same on Windows, Linux and macOS.
- **Settings:** `↑↓` move · `⏎`/space change · `←` / `→` turn a switch off / on and step through values · `/` filter the rows by name
  (`⏎` keeps the filter, `esc` clears it) · click the left half of a value or right-click to step back · the wheel scrolls the page ·
  `esc` back to the page you came from. The selected setting's description shows in a panel on the right on a terminal
  140 columns or wider, and under its row on a narrower one. Every row is also in the palette as `Setting: <name> (<value>)`.
- **System:** `↑↓` select · `c m p n` sort by CPU / memory / pid / name (again to flip) · `/` filter (or the
  "/ Filter" button) · `K` or `del` terminate (asks first; or the "K Terminate" button) · `esc` clears the filter, then back to Home.
- **Search:** type to find (case-insensitive) · `⏎`/`↑` older match · `↓`/`shift+⏎` newer · `esc` close.
  The newest 10,000 matches are kept (the bar then shows `10000+`).
- **Prompts and dialogs:** single-line prompts edit at a cursor (arrows, `home`/`end`, `del`, `ctrl+a/e/u`);
  confirmation dialogs need `y` (`n` or `esc` cancels), never `⏎`. `shift+⏎` sends a newline to AI CLIs
  (see [AI agents](#-ai-agents) for the terminals that report it). The pane menu (its "Launch in split ›" row lists the
  quick launchers) and the palette can launch an agent in a split.
- **Keys in a pane:** in a pane whose program asked for the kitty keyboard protocol, Alt with a shifted symbol such as
  `alt+!` is sent with that symbol's own code. On Linux and macOS, a terminal that answers neither the keyboard query
  nor the device attributes query can delay startup by up to 2 s (`TERM=dumb` is not asked).
- **Notifications:** clicking one that came from a pane (a toast, or `⏎` in the notification list) goes to that tab
  and pane; `ctrl+y` in the list copies its text. Background alerts also go to the outer terminal as OSC 9 / OSC 777 desktop notifications
  while NOBLE is not in front (on Windows only when `TERM_PROGRAM` is set; Windows Terminal gets the bell).
  While an agent waits for you, the window title reads `NOBLE · ◆ 1` with the number of agents waiting, and the
  terminal's progress report pauses (OSC 9;4: Windows Terminal, Ghostty and the like; Linux and macOS always, on Windows only in Windows
  Terminal or when `TERM_PROGRAM` is set). Both return to normal when nothing waits.
- **Mouse:** `ctrl+click` opens a URL (including OSC 8 hyperlinks) in the browser or a `path:line:col` in
  VS Code (else Cursor, Windsurf or Zed, whichever is on the PATH; else the default app); on macOS, iTerm2 and Terminal.app keep ctrl+click for their own menu, so use prefix `u`
  there · right-click a tab, pane title or project for a menu · the `+N` chip at the end of the tab bar lists the
  tabs that do not fit · drag tabs to reorder, double-click to rename, middle-click to close · the pane buttons `◫ ⊟ ⤢ ✕` (split right, split down, zoom, close) show on the focused
  pane and on the one under the mouse, and the status bar names the hovered one with its shortcut ·
  double-click a pane's title to zoom it (again to restore) ·
  drag dividers · drag to select text (copied on release), double-click a word, triple-click a line; the pane menu has Copy for a selection · right-click pastes (text with line breaks asks first unless the app uses bracketed paste; an app that captures the mouse gets the clicks, drags and wheel instead) · wheel scrolls ·
  scrolled back, click the position bar in the pane's last column to jump there, or the `↓ live` chip (any key
  typed into the pane works too) to return to the newest output; the wheel keeps scrolling over both ·
  `shift+drag` selects even inside apps that capture the mouse.

AltGr symbols (`@ { } [ ] \ | ~ €` on Turkish, German, Polish… layouts) are passed through as characters,
not as `ctrl+alt` chords.

</details>

## 🔧 Configuration

`noble --paths` prints the locations (Windows: `%APPDATA%\noble\config.toml`, data in `%LOCALAPPDATA%\noble`;
Linux: `~/.config/noble/config.toml`, data in `~/.local/share/noble`; macOS: both in
`~/Library/Application Support/noble`; `NOBLE_HOME` moves both). On Linux and macOS NOBLE creates its data folder
for your user only (0700) and its files 0600. The file is created with comments on first launch and **reloaded live** when saved.
Errors show up as a toast and never crash the app. A key or section that NOBLE does not read (a typo such as
`scrolback`) gets one warning toast that names it and its line, at startup and on each reload; clicking the toast opens
the file at that line. Sections NOBLE has removed (`[island]`) are ignored silently. Most options can also be changed
from the Settings screen.

<details>
<summary><b>Full <code>config.toml</code> reference</b></summary>

```toml
[general]
theme = "amber"          # any of the 22 themes in Settings
transparent = false      # let the terminal's own background (blur/opacity) show
boot_animation = true    # startup animation on the first launch and the first launch after an update (false: never)
animations = true        # page transitions, zoom, fades, tab and toast animations
animations_on_battery = true  # false: no animations while a laptop runs on battery
clock_24h = true
show_seconds = true
operator = ""            # name in the greeting; empty = your user name
check_updates = true     # look for a new release once a day and show it at the bottom right
show_tips = true         # hints in the status bar about the current screen (terminal, Home, System; none on Settings)

[terminal]
shell = ""               # empty = pwsh → powershell → cmd on Windows, $SHELL elsewhere
shell_args = []
scrollback = 5000
restore_session = true
resume_agents = true     # a restored Claude, Codex, Copilot, OpenCode or Antigravity pane picks its last conversation up again (false: starts fresh)
copy_on_select = true
osc52_write = true       # programs in a pane may set the clipboard (OSC 52, e.g. over SSH); false ignores them
tab_follows_cwd = true   # name a tab after its current folder (false: the folder it was opened in)
colors = "windows-terminal"  # windows-terminal (your PowerShell scheme; the theme elsewhere) | theme | campbell | dark-plus | light-gray | "wt:<your scheme>" …
background = ""          # override the scheme's background, e.g. "#c8c8c8"
foreground = ""          # override the scheme's text color
notify = true            # toast + bell when a background tab needs attention
notify_after = 10        # report background commands that ran at least this many seconds (0 = off)
dim_inactive = true      # in a split tab, fade the text of the panes without the focus
focus_mode = false       # hide NOBLE's top and bottom bars on terminal tabs (prefix f toggles it)

[keys]
prefix = "ctrl+a"
passthrough = "lock"     # "lock": prefix i locks a pane's keys to its app · "once": prefix + shortcut sends it
shell_first = true       # alt+. alt+, alt+t alt+s alt+p go to the shell in a terminal
[keys.prefix_bindings]   # key after the prefix → action ("none" unbinds)
"%" = "split_right"
[keys.direct_bindings]   # global chords
"alt+v" = "split_right"

[projects]
roots = []               # empty = Desktop, Documents, source/repos, projects, Developer, code, dev, src, repos …
max_depth = 4
exclude = []             # folder names the scan skips
# Projects added with A and removed with ⋯ → Remove from list are kept in state.json, not here.

[ai]
enabled = true
refresh_minutes = 5
providers = ["claude", "codex", "antigravity", "opencode-go", "kilo", "command-code"]
warn_at = 90             # warn once when a quota window reaches this percent, and when it resets (0 = off)

[[launchers]]            # Home quick launch; hidden when `command` is not on PATH
key = "c"
name = "claude"
command = "claude"
show = true              # false hides it from Home (Settings → Quick launch)
# Defaults: claude c, codex x, opencode e, copilot i, agy g (Antigravity), pi v, omp b
# (oh-my-pi), freebuff f, grok z (Grok Build), cursor-agent u, command-code d, cline l, kilo n.
```

**Actions** for bindings: `bridge system settings new_tab close_tab reopen_tab next_tab prev_tab tab_1…tab_9 move_tab_left
move_tab_right split_right split_down close_pane zoom equalize_panes swap_pane move_pane_to_tab focus_left focus_right focus_up focus_down focus_next
resize_left resize_right resize_up resize_down pane_menu palette help quit reload_config open_config cycle_theme
refresh_ai rescan_projects rename_tab save_workspace scroll_up scroll_down search focus_mode add_project_folder add_project
remove_project send_prefix passthrough jump_to_agent open_link update dismiss_update enable_hooks
dismiss_hooks_offer notifications prev_command next_command copy_last_output send_last_output`.

</details>

## 🧩 AI agents

NOBLE does not run an agent of its own. It hosts the ones you already use, shows what each one is doing and keeps
their conversations across restarts. The full reference (every agent's events and settings files, the states and the
keys) is in the [AI agents docs](https://noble.mertsoylu.dev/docs/ai-agents).

Shift+Enter starts a new line in Claude Code and Codex inside NOBLE instead of submitting the prompt. NOBLE passes
Enter on with its modifiers in the form each program reads: the kitty keyboard protocol for a program that asks for it
(Claude Code), console input records on Windows for a program that reads those (Codex, PowerShell). When a program asked
for the protocol but missed NOBLE's answer (Claude Code can, in its first-run folder-trust dialog), Shift+Enter still
goes in the kitty form. Ctrl+Enter is passed on the same way; what it does is up to the program. On Linux and macOS
NOBLE has to see Shift itself: it works when the outer terminal supports the kitty keyboard protocol (for example kitty,
Ghostty, WezTerm or foot), which NOBLE turns on at startup and off on exit. Any other terminal sends NOBLE a plain
Enter, which submits.

- **Real state for each agent:** Claude Code, Codex, GitHub Copilot and OpenCode report their prompts, answers,
  permission requests and, where the agent has them, errors, through their own hooks. Antigravity CLI and Cursor CLI
  report work and the end of a turn, but no permission requests (see their bullets below). Turn them on in
  **Settings → Terminal → Agent status hooks** (listed when an agent is installed here). The state shows on the tab
  dots, in each pane's title and on Home, and **prefix `a`** jumps to the next agent waiting for you.
- **Conversations come back:** with **Resume agent conversations** on (the default), a restored, saved or reopened pane
  whose launcher is an agent starts it again on its last conversation: `claude --resume <id>`, `codex resume <id>`,
  `copilot --resume=<id>`, `opencode --session <id>` or `agy --conversation <id>`. The id comes from that agent's hooks,
  so they must be on for it. A Cursor CLI pane starts a new chat: NOBLE keeps no id that `cursor-agent --resume` is
  confirmed to take.
  A shell pane where you typed `claude` by hand restores as a plain shell. The Home card's `↻ claude · 2h ago` line
  runs `claude --continue` in a project and needs no hooks.
- **Command blocks:** a PowerShell, bash 4.4+, zsh or fish pane marks where each command starts and ends, and NOBLE
  keeps the last 200 per pane. Jump to the previous or next command, copy the last output, or click a `✗` badge to go
  to a failed one. cmd.exe sends no marks, and bash 3.2 no command-start mark (OSC 133;C): these actions do
  nothing there.
- **Send to the agent:** **Send Last Output to Agent** (prefix `y`) pastes the selection, or else the newest command's
  output (at most 400 lines and 64 KiB), into the agent pane of the same tab (or else the one last sent to), led by a
  context line such as `~/app $ cargo test → exit 101`. It is pasted, never run: read it, then press ⏎ yourself. A multi-line paste into a
  shell that has not enabled bracketed paste asks first.
- **Progress:** a program's progress report (OSC 9;4, which Claude Code sends) shows in the pane title as `42%`, a
  spinner, `✗` on an error or `‖` when paused, and as a busy dot on the tab.
- **Background Claude sessions:** `claude --bg` sessions are listed in Home's AI usage panel, and a click attaches to
  one in a new tab (`claude attach <id>`). NOBLE lists and attaches only; it does not start them.
- **Outside NOBLE:** while an agent waits for you, the window title reads `NOBLE · ◆ 1` and the terminal NOBLE runs in
  gets a paused progress report (on Windows only in Windows Terminal or when `TERM_PROGRAM` is set). Background
  alerts also reach your desktop as OSC 9 / OSC 777 notifications while NOBLE is not in front (on Windows only when
  `TERM_PROGRAM` is set); **Notify from background tabs** (`notify`) switches them off.

<details>
<summary><b>Agent status hooks (optional)</b></summary>

<br>

Without help NOBLE can only guess whether an agent is busy. Turn on **Settings → Terminal → Agent status hooks**:
NOBLE opens a popup with one row per agent it found on this machine (**Claude Code**, **Codex**, **GitHub Copilot**,
**OpenCode**, **Antigravity CLI** and **Cursor CLI**; a row that is not found is shown as such). Click a row, or press
⏎ or space on it, to install or remove that agent's hooks. The first time you launch an agent that has no hooks yet, NOBLE offers to turn them
on for every agent it found. Installing keeps the hooks you already have and writes a backup of the settings file
the first time; turning the hooks off removes only NOBLE's entries (Copilot: its own `noble.json`; OpenCode: its
own plugin file).

| Agent | Settings file (Windows, Linux and macOS alike) |
|---|---|
| Claude Code | `~/.claude/settings.json` |
| Codex | `hooks.json` in `$CODEX_HOME` when set to an existing folder, else `~/.codex` |
| GitHub Copilot | `noble.json` in `$COPILOT_HOME/hooks` when set, else `~/.copilot/hooks` |
| OpenCode | `plugins/noble.js` in `$XDG_CONFIG_HOME/opencode` when set, else `~/.config/opencode` |
| Antigravity CLI | `~/.gemini/config/hooks.json` (its `noble` key; no documented variable moves it) |
| Cursor CLI | `~/.cursor/hooks.json` (NOBLE's entries in its `hooks` lists; the docs give this path for every platform) |

On Windows these are in your user profile (`%USERPROFILE%\.claude`, `%USERPROFILE%\.codex`, and so on). Each
agent runs `noble hook <event>` (with `--agent <id>` for the agents other than Claude Code) on the
events it reports: prompt, stop, permission request, notification, error, subagent start and stop, session start
and end. Codex has no notification or error event, OpenCode has no notification, subagent or session start and end
event, Antigravity CLI has session start, each model call and stop, and Cursor CLI has session start, each prompt (not
confirmed for the CLI), each tool call and stop. The command writes one small file per pane into NOBLE's data folder and exits; outside NOBLE
it writes nothing (Antigravity CLI and Cursor CLI still get their JSON reply on stdout).
The hooks apply to sessions started after the install: restart an agent that is already running.

- **Claude Code:** also reports a failed turn (`StopFailure`: an API error or a rate limit) as an error, and a permission request
  as "needs you" at once. The notifications name the request: "Claude asks to run Bash", "Claude asks a question".
- **Codex:** Codex runs user-level hooks only after you trust them once. After an install NOBLE says
  "Codex: open /hooks once to trust NOBLE's hooks"; open `/hooks` in Codex and trust them. NOBLE cannot do this for you.
- **GitHub Copilot:** NOBLE's hooks are in its own file, `noble.json`. Turning them off deletes that file when nothing else is in it; otherwise NOBLE removes only its own entries.
- **OpenCode:** a plugin forwards session status, idle, errors and permission requests to NOBLE, and it runs NOBLE
  only in a NOBLE pane. A child session is not told apart from its parent, so a parent tab can show idle while a
  child still works.
- **Antigravity CLI:** NOBLE shows working from each model call, idle when the turn stops, and an error when the turn
  stopped with one. agy has no permission, notification, subagent or session-end event, so an agent waiting for a
  permission answer still shows as working. NOBLE clears the pane's state when the shell prompt comes back. The resume id
  follows agy's hook documentation and binary.
- **Cursor CLI:** NOBLE shows working from each tool call, and from each prompt where the CLI sends that event; idle when
  the turn stops, and an error when the stop reports one. The CLI runs only some of Cursor's hook events, and none of the
  ones NOBLE uses reports a question or a permission prompt, so NOBLE never shows "needs you" for it: a turn that waits
  for an approval still shows as working. NOBLE registers no permission hook, so Cursor's own approval prompts are not
  touched. Which hooks run depends on the cursor-agent version: a turn with no tool call may not show as working at all.
  A restored Cursor pane starts a new chat.

The Home screen then lists every session as **working**, **needs you**, **your turn** or **error**, and a
background tab lights up the moment an agent asks for permission. A Claude session stays **working** while its
subagents (background ones too) still run, and Claude's idle reminder does not count as needing you. After you
answer a Claude permission prompt, the state stays "needs you" until the next prompt or stop. Hooks set by an
older NOBLE get the new events on the next start.

The pane title leads with the same state: `⠋ claude · working 2m +2` (a spinner, how long it has been working
and its running subagents), `◆ claude · needs you`, `● claude · your turn`, `✗ claude · error`, or `○ codex` for an
agent without hooks on, which NOBLE recognizes by name only. An agent without hooks reads as working while its
progress report (OSC 9;4) says busy work. Narrow panes keep the glyph and name, then the glyph
alone. Each tab starts with a dot for the most urgent state among its panes (◆ needs you, ✗ error, ● your turn,
spinner working, ○ running). The spinner
only turns while a working agent is visible, its pane is printing (an interrupted session stays "working" until
its next prompt, but stops spinning) and the laptop is plugged in; otherwise it is a still `…`.
**Prefix `a`** (Jump to Waiting Agent) goes to the next pane, across tabs, whose agent needs you, else to the
next one whose agent finished its answer, else to the next one whose agent failed.

</details>

## 🤖 AI quota

Usage is read from the logins that the official CLIs already keep on your machine. **Tokens are sent only to
their own provider, never displayed or logged, and never refreshed by NOBLE** (so it cannot race the CLI's
own token rotation). The last good numbers are cached and shown with `~` until the next successful fetch.

| Provider | Shown | Source |
|---|---|---|
| Claude Code | 5-hour and weekly limits, plan | `~/.claude/.credentials.json` → Anthropic OAuth usage endpoint |
| Codex / ChatGPT | 5-hour and weekly limits, plan | `~/.codex/auth.json` + `codex app-server` (`account/rateLimits/read`) |
| Antigravity | 5-hour and weekly limits (the fuller pool) | `agy --print /usage --output-format json`; only runs when an `agy` login exists |
| OpenCode Go | 5-hour, weekly and monthly limits | `~/.local/share/opencode/auth.json` → `opencode.ai/zen/go/v1/usage` |
| Kilo Code | Kilo Pass credits this billing period | `~/.local/share/kilo/auth.json` → `api.kilo.ai` `kiloPass.getState` |
| Command Code | 5-hour and weekly limits, plan | `~/.commandcode/auth.json` → `api.commandcode.ai/alpha/billing/credits` |

Only providers you are signed in to (and that have a quota plan) are shown. Quotas are fetched **only while
the Home screen is open**: right away when you come back to it (if the last fetch is 30 s or older) and then every `refresh_minutes`. While you work
in a terminal NOBLE makes no network calls. It also warns when, at the current pace, the 5-hour window would
fill up before it resets.

The same cached numbers drive three more warnings, with no extra request: an agent whose quota is full (100%) gets a
`⚠` on its launcher on Home, and launching it says when the quota resets (`resets in 1h12m`). An agent's pane title
shows the fullest window past `warn_at` (`⚠ 5h 92%`). When a window that passed `warn_at` resets, NOBLE says so
(`Claude Code 5h quota reset`) at the reset time it was given.

<details>
<summary><b>Shell integration</b></summary>

<br>

NOBLE learns each pane's working directory from OSC 7 / OSC 9;9, the last command's exit code from
OSC 133;D and when a typed command starts from OSC 133;C, with no setup:

- **PowerShell** (Windows, Linux and macOS): your existing prompt (oh-my-posh included) is wrapped to emit
  OSC 133;D (the exit code: 0, else `$LASTEXITCODE`, else 1) and OSC 9;9; your prompt still sees a failed
  command's `$?`. With PSReadLine (always there in an interactive session) the read-line call is wrapped to
  emit OSC 133;C before a line that is not blank runs.
- **cmd.exe**: a `PROMPT` that reports the directory, unless you already have one. cmd cannot report an exit
  code, so its panes show only how long a command took.
- **bash, zsh, fish** (Git Bash too): your own `~/.bashrc`, `.zshrc` or `config.fish` loads first, then a
  small hook reports the exit code (OSC 133;D) and the directory (OSC 7) on every prompt, and a typed command's
  start (OSC 133;C: `PS0` in bash 4.4+, `preexec` in zsh, `fish_preexec` in fish). Starship, oh-my-zsh and friends keep working, and so does a zsh
  `ZDOTDIR` of your own. A bash login shell (`-l` / `--login` in `shell_args`) loads `/etc/profile` and your
  `~/.bash_profile` (or `~/.bash_login` / `~/.profile`) instead of `~/.bashrc`, and `~/.bash_logout` on exit.
  On macOS bash, zsh, fish and PowerShell start as login shells, as in Terminal.app and iTerm2 (`/etc/zprofile`,
  `~/.zprofile` and Homebrew's PATH load); any `shell_args` of your own replace that. If a system `zshenv` sets
  `ZDOTDIR` outright, zsh cannot load NOBLE's hook and NOBLE says so; write it as `ZDOTDIR=${ZDOTDIR:-…}`. The hook scripts live
  in the data folder under `shell/`; pass `--norc` (bash), `-f` (zsh) or `--no-config` (fish) in `shell_args`
  to start the shell untouched.
- Any other shell whose prompt emits OSC 7 works too.

This is what lets splits open in the current directory and sessions restore where you left off. Each prompt (OSC 7, OSC 9;9 or OSC 133) also tells NOBLE
that the previous command finished: it refreshes that repository's git status and, for background tabs,
reports long-running commands. A background tab gets a marker in the top bar, after its title, for the most
urgent thing that happened there: `◆` for an OSC 9 / OSC 777 notification, the bell or an agent that needs you, `✗`
for a command that ended with a non-zero exit code, `✓` for a long command that finished, `•` for plain output. It
clears when you open the tab. A split tab shows its pane count (`⊞2`) when the strip has room (in a narrow window
only on the active tab), and the active tab is framed by accent bars (`▌work ×▐`). When tabs do not fit, the strip
slides to keep the active tab in view: `‹N` and `+N` count the hidden tabs before and after it, and each keeps
the most urgent marker of the hidden ones (`+3 ✗`).

The pane title shows the result of the last command typed at the prompt: `✓ 2.4s` when it worked,
`✗ 1 · 12s` with its exit code when it failed, `· 12s` in cmd.exe. It stays until the next command starts. A
command that runs longer than 2 s shows a live timer there instead (`◷ 0:07`, then `◷ 3m` updated once a
minute; on battery the first minute shows `◷` alone, so an idle pane still redraws at most once a minute).
A program can also report its progress with OSC 9;4 (Claude Code does, and so do other tools that use ConEmu's
form). The pane title shows it as the percentage (`42%`), a spinner while busy work has no percentage, `✗` on an
error and `‖` while paused. While it reports busy work, the tab's dot shows the working glyph (the spinner, or a still
`…` on battery), also over an idle agent's `○`. The report ends with state 0 or with the shell's next prompt.
The command start mark (OSC 133;C) tells a command that ran from an Enter on an empty, cleared or
continuation line, which records nothing; the time counts from when the command started, not from the first
line typed. cmd.exe and bash 3.2 (macOS `/bin/bash`) send no such mark: there the last Enter starts the
clock and a typed line counts as a command (a line typed and cleared again may repeat the last result).
On a narrow pane the title drops its folder first, then the tag, then this badge.

</details>

## 🩺 Troubleshooting

Run `noble doctor` for a plain-text report of your setup (version, OS, outer terminal, shells and their integration,
agent status hooks per agent, AI provider detection without any token, config and data paths, `gh` state). It works the same on
Windows, Linux and macOS, never opens the terminal, and is what a bug report asks for.

- **Splits and restored tabs open in the wrong folder:** the shell is not reporting its directory (see Shell
  integration above). cmd keeps a `PROMPT` you set yourself, so add `$E]9;9;$P$E\` to it; bash, zsh and fish
  started with `--norc`, `-f` or `--no-config` skip the hook; any other shell needs a prompt that emits OSC 7.
- **An AltGr symbol runs a NOBLE shortcut instead of typing:** a character that arrives with Ctrl+Alt (how
  Windows reports AltGr) is treated as text, and on Linux the terminal sends the character itself. If a key
  still triggers a shortcut, the terminal sends it as an `alt+` chord: unbind it (`"alt+…" = "none"` under
  `[keys.direct_bindings]`) or lock the pane's keys with prefix `i`.
- **`alt+` shortcuts type `π`, `¡` or `º` on macOS:** the terminal sends Option as a character. Turn on
  Option as Meta: Terminal.app Settings → Profiles → Keyboard → "Use Option as Meta key"; iTerm2 Profiles →
  Keys → Left Option key → Esc+; Ghostty `macos-option-as-alt = true`; Kitty `macos_option_as_alt yes`;
  Alacritty `option_as_alt = "Both"`. On layouts that type `@ { [ |` with Option, set it for the left Option
  key only. The prefix (`ctrl+a`) works without it.
- **The project list is empty on macOS:** macOS asks your terminal app for access to Desktop and Documents the
  first time NOBLE scans them. Allow it (System Settings → Privacy & Security → Files and Folders), or set
  `projects.roots` to folders outside them.
- **Colors look wrong or washed out:** NOBLE draws in 24-bit color. Use a truecolor terminal (Windows Terminal,
  WezTerm, Kitty, GNOME Terminal, Konsole, iTerm2 …) rather than the old Windows console, the Linux text console
  or Terminal.app before macOS 26; in
  tmux add `set -ag terminal-overrides ",*:RGB"`. Panes get `COLORTERM=truecolor`; their palette is Settings →
  Terminal colors (`terminal.colors`).
- **Claude Code's fullscreen view shows stale or misplaced text:** Claude Code's fullscreen view sends only the cells
  that changed between frames, and some ConPTY-backed hosts (Windows Terminal among them) can leave fragments of
  earlier output on screen until the window is resized. NOBLE's tests of full and partial redraws, resizes and
  synchronized blocks in a Windows (ConPTY) pty found no such leftovers, so the problem was not reproduced here. The
  workarounds Claude Code documents: start it with `CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT=1` (repaint every cell on every
  frame; PowerShell `$env:CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT = "1"`, cmd `set CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT=1`,
  fish `set -x CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT 1`, bash and zsh the same prefix form before `claude`), press
  `Ctrl+L` to redraw the screen, or run `/tui default` (or set `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1`) to use the
  classic renderer.
- **Copy does not reach your clipboard over SSH:** with no system clipboard NOBLE sends the text with OSC 52,
  which the terminal on your own machine has to allow (on by default in Windows Terminal, WezTerm and Kitty;
  tmux needs `set -g set-clipboard on`).

## 🔋 Performance

There is no fixed frame rate. NOBLE redraws when an event changes something visible, at most 60 times a second
under heavy output. Everything else runs only while it is on screen:

- System sensors: every second on the System screen and on Home (every 2 s on battery), every 5 s elsewhere.
- Git status: when a command finishes, when Home opens, or when a repository changes.
- Listening ports and the pull request / CI state of the selected project: only while Home is open. Ports are
  scanned every 10 s (30 s on battery); the PR is asked for only when the selected project's branch changed or
  its answer is older than 60 s (180 s on battery).
- AI quota: only while Home is open.
- On battery the Home clock drops its seconds and the blinking colon, and the cursor of the command palette
  and prompts stops blinking.

## 🤝 Contributing

Bug reports, ideas and pull requests are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for the development
setup, the architecture overview and the test suite (headless render snapshots at seven terminal sizes, real
PTY sessions and end-to-end tests of the compiled binary). Security issues: see [SECURITY.md](SECURITY.md).

## 📄 License

[MIT](LICENSE) © NOBLE contributors
