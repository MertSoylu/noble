# Changelog

All notable changes to NOBLE are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Settings → Key bindings (in the Keys section) edits every shortcut from the screen: a list of all actions with
  their prefix and direct keys, ↑↓ to select, `/` to filter, `⏎` to rebind the prefix key, `d` for the direct chord,
  `x` (or del) to unbind and `r` to restore the default. A key another action already has is asked about first
  ("ctrl+a n is Next Tab — replace? y/n"); keys NOBLE cannot bind (the prefix itself, ctrl+alt with a symbol, a direct key
  without ctrl or alt such as a letter, Enter or an arrow, except F1–F12) are refused with the reason. Each change is written to `config.toml` under its table, the
  rest of the file and its comments are kept, and the keys reload at once. Clicking a row selects it; double-clicking
  starts the rebinding. The `?` reference points to it.
- Home's project card lists the selected project's quick tasks as a TASKS row of clickable chips (`▸ npm run dev`)
  when there is room. Tasks are read from the project folder: the `package.json` scripts (run with npm, pnpm, yarn
  or bun, by the lockfile that exists), `cargo build`, `cargo test` and `cargo run` for a Cargo project, the plain
  Makefile targets and the justfile recipes (at most 12 per project). A chip runs its command in a new tab in the
  project folder, named "project · command". Home's `T` (shift+t) opens the palette with the project's tasks ("Run: npm run
  dev", group TASK), shown only when the project has tasks; the hint bar shows `T run task` then too, unless a
  launcher uses `T` (launchers are checked first and keep the key). The tasks are
  detected by the project scan, so a new script shows up on the next rescan.
- Closing a terminal tab (close tab, its last pane, middle-click or ×) can be undone: prefix `T` (Reopen Closed
  Tab, `reopen_tab`) brings back the last ten closed tabs, most recent first, with their split layout, each
  pane's directory, launcher and name, at their old index and focused. The tab-closed notice names the key; the
  palette lists the action only when there is something to reopen. The history is kept in memory, not saved,
  and a shell that exits on its own is not kept.
- Home has one add key: `a` opens the add prompt in auto mode, which decides on Enter. A git repository (or a
  folder with no repository within two levels) is added as a project ("added project X"); a folder with git
  repositories inside is scanned for them ("scanning X for projects"). `tab` cycles auto, folder to scan and one
  project. `A` still opens the prompt in project mode, and the palette's "Add Project…" still works.
- Split tabs fade the text of the panes without the focus (`terminal.dim_inactive`, Settings → Terminal → "Dim
  inactive panes", on by default): their colors are mixed 30% toward the pane background, so the focused pane
  stands out. A zoomed pane is alone and not dimmed; ANSI colors that the palette leaves to the outer terminal
  get the terminal's dim attribute instead.
- The keyboard reference (`?`) is interactive: typing filters its rows by key or description (the title shows
  `/query`; sections without a match are hidden), ↑↓ select a row and ⏎ or a click runs its action and closes
  the reference, and esc clears the filter before it closes. Wide screens lay the sections out in two columns
  (from 120 columns) or three (from 170), each section kept whole; a narrow screen keeps one scrolling column.
- The command palette shows which letters of each title the query matched, in bold accent (the title is cut
  with "…" as before).
- The palette lists the last ten commands run from it first, most recent first, when the query is empty; the
  list is kept in `state.json` (`palette_recent`).
- Every Settings row is in the command palette as `Setting: <name> (<value>)` (group SET): running it does what
  ⏎ does on that row, and a popup row (theme, terminal colors, quick launch) opens its picker.
- Settings filter: `/` on the Settings page narrows the rows by name (the query shows in the frame's title,
  `esc` clears it, and "no settings match" shows when nothing fits); the status bar lists "/ search".
- Prefix `u` (Open Link on Screen, `open_link`) lists the URLs and `file:line` links on the focused pane's
  screen, newest first, and opens the chosen one: the keyboard way to ctrl+click, which iTerm2 and Terminal.app
  on macOS keep for their own menu.
- Home's project card shows the selected project's pull request and CI state (`PR #42 open · ✓ CI 12/12`, read
  with the GitHub CLI `gh`; needs `gh` installed and `gh auth login`) and the TCP ports its running processes
  listen on (`⇄ :3000 :5173`). Click the PR to open it in the browser, a port to open http://localhost:<port>.
  Without the card (small terminals) both move to the footer line.
- The tab bar's "+N" chip opens the command palette listing only the tabs, for the tabs the bar has no room for.
- Actions `enable_hooks` and `dismiss_hooks_offer`: the one-time Claude status offer can be accepted or
  dismissed from the palette while it shows.
- Action `notifications` (Show Notifications) lists the last 50 notifications in the command palette, newest
  first, with their time; choosing one copies its full text.
- Setting `general.show_tips` (Settings → Show tips, Display) turns the status bar tips off.
- The first time a shell key (`alt+t`, `alt+p` …) goes to the shell in a terminal although NOBLE has a shortcut
  for it, the status bar says once which chord reaches that action (`alt+t went to the shell · NOBLE: ctrl+a t`;
  when the action has no prefix chord, it points at Settings → Leave shell keys to the shell).

### Security
- Background git (the project list and Home's status) no longer runs programs a repository's own `.git/config`
  names: a `core.fsmonitor` hook, a `filter.*` clean, smudge or process command, or `log.showSignature`'s
  `gpg.program`. Such a repository shows no git status instead of running them. Filters from your global config
  (git-lfs, for one) still apply.
- A path the terminal reports as a network path (`\\server\share`, `//server/share`) is never checked on disk: an
  OSC 9;9 report of one is ignored (the pane keeps its start folder), and a Ctrl+click on one does not open or
  check it. On Windows such a check connected to the server and sent the user's credentials.
- A pane opened in a network folder no longer resolves a relative link against that folder on the share (ctrl+hover
  and ctrl+click), and its prompts ask for no git status there. A repository whose `.git` file, `commondir` or
  `gitdir` names a network path is listed as a plain folder: nothing is read from the share.
- The link opener refuses more program types (Python scripts, `.msc`, `.jnlp`, `.scf`, `.gadget`, Windows Script
  Host and installer types, `.library-ms`, `.settingcontent-ms` and others) and copies them instead of opening them.
  Names Windows reads as a program with a trailing dot or space (`setup.exe.`) or as a stream
  (`setup.py::$DATA`) are copied too.
- `noble update` downloads only https addresses on GitHub, caps the download sizes, and refuses an archive with
  absolute names, `..` parts or links before unpacking anything. A bad checksum now reads as a corrupted or
  incomplete download. Redirects are followed by NOBLE itself, each address checked (GitHub's own download hosts
  only), not by the HTTP client. `tar` is run by its full path, never found in the working folder.
- NOBLE's data folder is created 0700 and its files 0600 on Unix (existing files keep their mode). Without a home
  folder NOBLE stops with a message and asks for `NOBLE_HOME`, instead of writing to a `.noble` folder in the working
  directory.
- The `noble hook` command in `~/.claude/settings.json` is quoted for the shell on Unix (single quotes): a `$` or a
  backtick in the install path no longer expands. An install from an older version gets the events it lacks in the
  new form (its existing entries keep theirs until the hooks are turned off and on again).
- Programs in a pane can set the clipboard with OSC 52 as before; Settings → "Clipboard from programs" (or
  `osc52_write = false` under `[terminal]`) ignores those requests.

### Changed
- Home's "/ search" label moved from the Projects panel's bottom button row to its top border, next to the
  count ("/ search · 6"); a click still starts the filter.
- The add prompt replaces the separate `a` (folder to scan) and `A` (one project) choices: the status bar shows
  "a add", the help and the empty-state texts mention only `a`, and the palette's "Add Project…" has no key hint.
- The confirmation toasts say what was added: "added project X" or "scanning X for projects".
- Settings on a terminal 140 columns or wider: the page widens to 130 columns, the list keeps its first 80, and a
  panel on the right shows the selected setting's label in bold with its description wrapped. Narrower screens
  show the description dimmed under the selected row (it used to sit on the bottom row, far from the selection).
- In the keyboard reference `j`, `k`, space and `q` type into the filter: ↑↓ and PgUp/PgDn move the selection
  (the list follows it), the wheel scrolls, and `?` (with no filter typed) or esc closes it.
- The startup animation plays on the first launch and on the first launch after an update, not on every launch.
  `general.boot_animation = true` keeps that behavior, `false` turns it off for good; `--no-boot` still skips it.
- The status bar tips fit the screen: terminal tips in a terminal, Home's on Home, process list tips on System, none
  on Settings. A tip changes when the screen changes instead of every 20 seconds.
- The terminal hint "drag border resize" appears only on a tab with more than one pane.
- Error notifications stay until they are clicked away; the others leave after 3 s (4 s for warnings). The pointer
  on a notification pauses its countdown, and a click dismisses it.
- A config error gives its line and column (`config line 5, column 11: …`), and clicking that notification opens
  the config file at the line.
- Closing a tab with several panes asks first, even when they are idle.
- The amber theme's `warn` color is a red-orange instead of a second orange, so git's "changed" state stands out
  from the accent. Everforest's `warn` moved from yellow to its orange for the same reason.
- Removing a project from the list asks first (`A` on Home adds it back).
- Saving a workspace under a name that is already taken asks before replacing it; a blank name keeps the prompt
  open with an error.
- System's Processes panel has "/ Filter" and "K Terminate" buttons; its status bar shows only the keys the
  panel does not show.
- Home's "/ search" label starts the project filter when clicked.
- One name per feature: "Pass Keys to the App" in the palette and "? keys" in the hints.
- Search Scrollback and Pass Keys to the App are listed in the palette only in a terminal, where they apply.
- The top bar's tab strip slides to keep the active terminal tab visible: tabs hidden before it are summarized
  as a "‹N" chip (carrying the most urgent marker, like "+N"). Home drops the small clock (the big one is
  there), and under 80 columns Home and System are single glyphs (⌂ and ▤) to make room for tabs.

### Fixed
- Scrollback search keeps up with streaming output without slowing it: while the search bar is open the matches are
  recomputed at most every 100 ms (the output that arrives in between is counted when the interval has passed), and
  a pane resize recomputes them at once. The matches are capped at the newest 10,000, shown as "10000+" in the bar,
  so a common letter over a long scrollback no longer builds a huge list.
- The ctrl+hover highlight of a link no longer sticks: it ends with the next key or mouse event that does not hold
  ctrl (or a released ctrl, where the terminal reports it), and output that moves the link moves the highlight with
  it. A ctrl+click opens the link that is highlighted.
- Files NOBLE writes survive a power loss: the new content is flushed to disk before it replaces the old file
  (and the folder is flushed on Linux and macOS), so a crash leaves the old or the new file, never a truncated one.
  Write errors are no longer ignored before the rename. A symlinked file is still written through the link, and a dangling
  link is reported instead of being replaced by a regular file. On Windows a save retries briefly when another
  program has the file open.
- A Settings change (theme, launcher, scheme, prefix key, roots) is refused while `config.toml` does not parse, with
  a toast that opens the broken line; it used to rebuild the file from the defaults and wipe the `[[launchers]]`
  blocks. A config file that is a link to a missing file is reported at startup.
- Saved tabs that could not start at all (no pane could be spawned) are kept in the session file for the next start
  instead of being replaced by the tabs that did open.
- A periodic session save that cannot get the session lock (another window holds it) is retried later, instead of
  writing without the lock and possibly losing another window's tabs. A stale lock is taken over safely, and a
  window only removes its own lock.
- Damaged-file copies rotate: past five copies the oldest is dropped, so a file that keeps coming back no longer
  stops NOBLE from saving it.
- `state.json`, `session.json` and workspace entries keep the fields a newer build wrote when an older build saves
  them again.
- Claude hooks: uninstall removes only NOBLE's hooks, and keeps the user's own hooks in a matcher group that also
  holds one of NOBLE's. A backup cut short by an interrupted copy is taken again, and the backup is written in one
  step.
- Updating NOBLE on Linux and macOS replaces the binary in one step (no moved-aside copy that a kill could leave
  without a `noble`).
- Scrollback search no longer copies the whole scrollback on every keystroke, and only copies it again (holding the
  terminal's lock) when the pane has new output or was resized.
- A damaged `state.json`, `workspaces.json`, `session.json`, `recent.json` or `ai-history.json` no longer loses its data on the
  next save: its bytes are kept in `<name>.corrupt` (then `.corrupt.2` … up to five copies; identical bytes are kept once)
  before the write, and a toast names the copy. A file that cannot be read, or whose copy cannot be made, is left alone and
  the change is not saved; an empty file is an empty state. A failed save is reported once per run.
- A config error toast goes away when the file loads again, and opening the notification list clears the toasts on screen.
- The key editor refuses direct ctrl+letter shortcuts (ctrl+c, ctrl+d, ctrl+r … belong to the shell), keeps CRLF line endings
  in `config.toml`, and adds a key before a comment that titles the next table. Reopening a tab says its launch commands run again.
- Adding a folder on Home checks the folder on a helper thread, and project files read for quick tasks are capped at 256 KiB.
- Resetting a shortcut (`r`) keeps a key that another action has taken since, and says so ("ctrl+a t stays on
  Zoom"). Clicking a notification dismisses that notification even when another one left the screen first.
- The key editor refuses to write while `config.toml` does not parse (the error names the line), and refuses a
  bindings table written inline or as dotted keys rather than adding a second one. Spaces inside table headers are
  accepted.
- The startup animation plays once for `noble` and once for `noble-dev`: alternating between them no longer replays it.
- Adding a folder in auto mode no longer freezes the screen while its subfolders are checked: a folder that does
  not answer within two seconds is added as a project.
- The README terminal screenshot shows the full pane path (`D:\dev\noble`); the screenshot generator wrote only as
  many characters as the text it replaced, which cut the path short.
- The active terminal tab no longer hides inside the "+N" chip when there are more tabs than fit in the top bar.
- Key hints follow the keymap: the help's tab rows, the welcome screen's palette key, the terminal status bar's
  home key and the status-bar tips show rebound keys and leave out unbound ones instead of the defaults.
- Platform text: the terminal scheme preview shows a Unix prompt on Linux/macOS instead of `PS C:\>`, and the
  color-scheme tip mentions Windows Terminal only on Windows.
- Palette hints match the view: in a terminal they show the keys that reach NOBLE there (none while the pane's
  keys are locked); on Home they show the single keys Home handles itself (`r`, `R`, `a`, `A`).
- Home keeps the "esc clear" hint while a kept filter is non-empty, even when it matches nothing.
- Below 50 columns the status bar still shows the palette key as a chip.
- Home's `o` (and the Folder button) with no project selected says "select a project first" instead of doing
  nothing.
- Help says "enlarge window" when the terminal is too small to show it.
- Confirmation dialogs wrap long text instead of cutting it off.
- Dialogs next to a panel frame no longer show doubled corners at small sizes.
- A pane opened while another pane or window was writing NOBLE's shell scripts (the first start, or the
  first start after an update) could start without the shell integration: no directory tracking, no exit
  codes. The scripts are now replaced atomically.
- Terminating a process (System's K or its dialog) checks that it is still the process that was listed (its start
  time): when its pid has been reused by another process, nothing is terminated and a notice says so.
- The codex usage check ends its whole process tree when it times out or fails (on Windows the npm `codex.cmd` shim
  used to leave the CLI running), and it is ended on shutdown like the other background commands.
- Quitting ends the background commands before it saves the session, and a crash on the main thread still saves the
  session and ends them.
- A dead folder in the recent list is checked once per run: each new tab no longer stalls up to half a second on it.
- A git refresh that was requested while a project rescan was queued is no longer lost.
- On Windows one slow background command kill no longer holds up every other background command.

## [1.9.0] - 2026-10-06

### Added
- Windows ARM64 release archive (`noble-windows-aarch64.zip`); `noble update` picks it up.
- Terminal.app before macOS 26 (no truecolor) gets the nearest 256-palette colors instead of garbled ones.
- `~/Developer` and the localized XDG desktop/documents folders are default project roots.
- A warning when the system `zshenv` sets `ZDOTDIR` and so keeps NOBLE's zsh integration from loading.
- More animations: overlay backdrops fade in and list selections glide; notifications slide in and out
  with a countdown line; tabs grow open and close up, a background tab flashes when it gets a marker;
  a new split unfolds and the focused pane's frame lights up; Home fades in row by row, its bars and
  percentages slide to new values, an agent that changes state lights up its row and hover highlights
  fade in. On laptops, "Animations on battery" in Settings (`animations_on_battery`) turns them off on
  battery.

### Changed
- On battery the cursor of the command palette and prompts stays lit instead of blinking, so an open
  palette no longer redraws the screen twice a second.
- Ctrl+click never runs a program: a link to an executable, script, app bundle or installer copies its path.
- `ctrl+alt+<non-letter>` key bindings are refused with a warning (they are AltGr on Windows) instead of
  silently binding the bare key.
- Network rates leave out loopback and virtual links (VPN tunnels, container bridges); disk rows leave out
  container, snap and simulator mounts and list a device once.
- Opening a folder, file or link over SSH no longer starts it on the remote screen on Windows and Linux;
  the clipboard falls back to OSC 52 there (a forwarded X11 display still counts as the user's).

### Fixed
- `noble update` works in a fresh private temp directory (shared `/tmp` on Linux could be abused by
  another local user).
- Windows: a multi-line paste with the terminal's shortcut (Ctrl+V) asks first and uses bracketed paste.
- Windows: the session is saved on logoff and shutdown; the release binary no longer needs the VC++ runtime.
- Windows: Git Bash directories use backslashes ("Open folder" opened Documents), network shares work in Git
  Bash and cmd.exe, and MSYS2/Cygwin zsh and fish report Windows paths.
- Windows: `bash` that is WSL's launcher is started without NOBLE's bash script (it hid `~/.bashrc`);
  PowerShell arguments with their own `-Command`/`-File` are kept; `WT_SESSION` no longer leaks into panes;
  `install.cmd` works with several noble-dev windows open.
- Linux: a battery reporting `Unknown` on AC no longer turns on battery mode; a hung network mount no longer
  freezes CPU and memory readings; programs NOBLE opens survive closing its terminal window and a failed
  `xdg-open` is reported.
- Saving `config.toml` or Claude's `settings.json` keeps a symlinked file a symlink and keeps its permissions.
- A remote shell's OSC 7 (after `ssh` from a pane) no longer changes the pane's local directory.
- macOS: background git no longer pops up the Command Line Tools installer; PowerShell starts as a login
  shell like the other shells; zsh launcher commands read `~/.zlogin`; `~/Projects` keeps its on-disk case.
- Tests and the screenshot example no longer leave a `noble-headless` folder in the temp dir.
- bash login shells read `~/.bash_logout` on exit; `install.sh`/`install.cmd` respect `CARGO_TARGET_DIR`.

## [1.8.0] - 2026-09-30

### Added
- `noble <folder>` (for example `noble .`) opens NOBLE with a shell tab in that folder.
- The session is saved a couple of seconds after tabs, splits or directories change, and also when the terminal
  window is closed, the process gets SIGHUP/SIGTERM, or Windows logs off or shuts down.
- Saved workspaces can be deleted from the command palette; saving says when it replaced a workspace or dropped
  the oldest one at the limit of 20.
- Launch an AI agent in a split next to the focused pane (pane menu and palette).
- Background notifications (agent needs you, done, long command finished) are also forwarded to the outer
  terminal as OSC 9 / OSC 777 desktop notifications when NOBLE is not in front.
- One-time offer to enable Claude status hooks after the first Claude launch.
- Double-click selects a word, triple-click a line, and the pane menu has Copy for the current selection.
- Narrow Home (under 92 columns) shows a one-line AI quota strip and the number of agents that need you.
- Home keeps a provider whose AI login expired, showing its cached usage dimmed with a "run <cli> to refresh" hint.
- Settings: a short description of the selected setting; click the left half of a value or right-click to step back.
- Single-line prompts get cursor editing (arrows, home/end, delete, ctrl+a/e/u) and placeholders.
- Commands with nothing to act on now say why in a toast.

### Changed
- Shift+Enter sends ESC CR so AI CLIs insert a newline (needs an outer terminal that reports Shift).
- Confirmation dialogs no longer accept Enter; press y to confirm, n or esc to cancel.
- The add-project prompt validates on Enter and keeps your text when the path is wrong.
- On Home the status bar keeps "? help" longest, and its "commands" chip opens the palette when clicked.
- Palette launchers in a terminal use the focused pane's directory.
- The welcome card lists launcher keys only for installed CLIs, and System calls the kill key "terminate".
- Home computes agent sessions once per frame instead of once per project row.
- Claude hooks setup keeps the first backup of `settings.json` and only recognises NOBLE's own hook commands.
- Background git status no longer takes the index lock, never prompts for credentials, and is killed if it hangs.
- A dead network share can no longer freeze the UI: working directories and recent folders are checked once
  with a time limit instead of on every frame or new tab.

### Fixed
- Saving a setting no longer turns a multi-line array such as `roots = [...]` in `config.toml` into a broken
  file, and config saves are atomic.
- ctrl+click no longer opens links with unknown schemes from terminal output; the address is copied instead.
- Linux battery readout ignores wireless mice and other device batteries.
- Programs NOBLE launches (browser, editor, file manager) no longer linger as zombie processes on Linux and macOS.
- Saved workspaces and recent folders from several open windows no longer overwrite each other.

## [1.7.0] - 2026-09-30

### Added
- Pane titles show how the last command ended: `✓ 2.4s`, or `✗ 1 · 12s` with the exit code when it failed
  (cmd.exe cannot report a code, so only the time shows there). A command running longer than 2 s shows a
  live timer instead (`◷ 0:07`, updated every second in its first minute and once a minute after that or on
  battery). NOBLE's PowerShell, bash, zsh and fish hooks now report the exit code with OSC 133;D on every
  prompt, and mark a typed command's start with OSC 133;C (bash 4.4+ `PS0`, zsh `preexec`, fish
  `fish_preexec`, PowerShell through PSReadLine). Only that mark starts the clock and counts a command, so an
  Enter on an empty, cleared or continuation line, the shell's first prompt and an Enter typed into a running
  command do not count, and a multi-line command is timed from when it runs. cmd.exe and bash 3.2 have no such
  mark: there the last Enter starts the clock. When a prompt reports two codes, the first (NOBLE's) wins.
- Agent state in the terminal: a pane running an AI agent leads its title with its state (`⠋ claude · working
  2m +2` with the running subagents, `◆ codex · needs you`, `● claude · your turn`, `○ claude` when unknown),
  shortened on narrow panes, where the folder goes first. Each tab starts with a dot for the
  most urgent state among its panes. The spinner only redraws (every 130 ms) while a working agent is in a
  visible pane that printed something in the last 3 s and the laptop is plugged in; otherwise (on battery, or
  a session left "working" by an interrupt) it is a still `…`. A changed Claude hook state now
  redraws the terminal and the tab strip, not only Home.
- New action `jump_to_agent` (Jump to Waiting Agent, prefix `a`): goes to the next pane across all tabs whose
  agent needs you, else to the next one whose agent finished its answer, restoring a zoomed tab that shows
  another pane; says "no agent is waiting" otherwise.

- Typed tab markers in the top bar: `◆` notification, bell or agent needs you, `✗` a background command that
  failed (non-zero exit code; the toast names the code when the command ran long), `✓` a long command that
  finished, `•` output. The marker has its own cell after the title instead of overwriting its last letter, the
  most urgent kind wins, and `+N` for tabs that do not fit keeps the most urgent hidden marker. The tab's
  agent dot no longer repeats a `◆` notice.
- A split tab shows its pane count (`⊞2`) after the title when the strip has room (only on the active tab in
  compact mode), and the active tab is framed by accent bars (`▌work ×▐`) instead of relying on color alone.
- Scrollback helpers, shown only while a pane is scrolled back: a position bar over the content's last column
  (thumb sized by the visible share, search matches marked `•`, the selected one `●`; click it to jump) and a
  `↓ live` chip at the bottom right, above the search bar, that counts the lines which arrived meanwhile
  (`↓ live · 12`) and returns to the newest output when clicked. Small panes get only the chip. The status bar
  says how to go back while the focused pane is scrolled back. The wheel keeps scrolling over the bar and the
  chip, and a right or middle click there pastes as on the content.
- Double-click a pane's title to zoom it (again to restore); middle-click a tab to close it (it asks first
  while something still runs there, like the `×`).

### Changed
- Pane title buttons: one bracketed cluster on the title line, `┤ ◫ ⊟ ⤢ ✕ ├` (split right, split down, zoom or
  `◱` restore, close), shown on the focused pane and on the pane under the mouse; other panes keep a plain line
  there and have no hidden click targets. A hovered button turns into a small inverted chip (close in the error
  color), readable in every theme, and the status bar names it with its shortcut (`◫ split right · ctrl+a v`).
  On a line two panes share, only the pane whose title row it is shows its buttons. When a title is narrow it
  keeps, in this order, close, the key lock and agent glyph, the first letters of the name (`po…`), zoom and
  the splits, the rest of the name, the agent state or last command result, the `KEYS`/`↑N`/`ZOOM` tag, and
  last the folder, so even a small pane in a grid keeps its name and lock.
- The first launch welcome card is now a short setup: pick the theme and terminal colors (the whole UI previews
  them as you cycle with ←→ or click ‹ ›), the shell when more than one is installed, and the prefix key. ⏎
  saves the choices, esc keeps the defaults and undoes the preview. On a small window the setup rows stay
  visible and the key list is dropped.

### Fixed
- PowerShell: a prompt that shows the last command's status (oh-my-posh, the Windows Terminal shell
  integration snippet) saw every command as successful inside NOBLE, because NOBLE's prompt wrapper reset `$?`.
- Claude Code status: a background tab now gets its "Claude finished" notice when the last subagent ends after
  Claude's answer (it was lost before). Hooks installed from a path that contains " hook " are upgraded correctly.
- Several NOBLE windows no longer undo each other's removed, added or pinned projects: every `state.json` write
  re-reads the file under a lock and changes only its own field.
- A project added by hand no longer vanishes when a scan that started before the add finishes.
- Tab menu, rename and close work on the right tab after the pane they were opened from closes in a split tab,
  and a launcher tab's title drops the launcher name once that pane is gone.
- Search stays fast while a command prints a lot with the search bar open, and a failed atomic write (state
  files, Claude Code's `settings.json`) no longer leaves a temporary file behind.
- The theme and terminal color selectors start over from the file when `config.toml` changes while they are
  open, so esc no longer puts the old values back.

## [1.6.0] - 2026-09-29

### Added
- Home: remove a project from the list for good with ⋯ → **Remove from list** (or "Remove Selected Project
  from List" in the command palette). It stays hidden after a rescan and a restart, is unpinned, and the folder
  itself is not touched.
- Home: `A` adds one folder as a project ("Add Project…" in the palette), even outside the scanned folders, too
  deep for the scan or without git (listed as "no git"). It stays listed after rescans and restarts, and adding
  a removed project brings it back. In the add prompt `tab` (or a click) switches between one project and a
  folder to scan (`a`). Both lists live in `state.json`; a folder that no longer exists is skipped.

### Changed
- A new, calmer boot animation (3 s): viewfinder brackets open from the centre, particles scattered over the
  screen fly into the NOBLE logo and each cell decodes through random glyphs before it locks in. At the end the
  logo dissolves and the brackets open out to the screen edges, revealing the app. Only the logo is shown: the
  tagline, checklist, progress bar and "PRESS ANY KEY" are gone. Any key or click still skips it.

### Fixed
- The AI collector no longer busy-loops while it is disabled or hidden, and `ai-cache.json` is written atomically.
- The tab menu and rename prompt track tabs by pane id, so a closed tab can no longer leave a stale index or panic;
  empty menus no longer underflow. Agent state changes redraw Home, palette tab hints follow key rebinds and
  folder names containing " · " keep their tab title.
- Bracketed paste strips ESC from the payload, drifted OSC 8 links no longer resolve to the wrong URL, and search
  keeps its match once the scrollback is full.
- Claude Code status hooks: a session whose subagents (background ones too) are still running stays
  **working** instead of turning into **needs you** / **your turn** when Claude's main answer ends, and no
  "Claude finished" notice fires until Claude answers again. Claude's idle reminder ("Claude is waiting for your
  input") and a background subagent finishing no longer count as needing you; permission requests still do.
  NOBLE now also listens to `SubagentStart` / `SubagentStop`; hooks set by an older NOBLE get them added on
  the next start.

## [1.5.0]

### Added
- macOS support (Apple Silicon and Intel): release binaries (`noble-macos-aarch64.tar.gz`,
  `noble-macos-x86_64.tar.gz`, with `.sha256`), `noble update`, and the full test suite in CI.
  - bash, zsh and fish start as login shells, as in Terminal.app and iTerm2, so `/etc/zprofile`, `~/.zprofile`
    (Homebrew's PATH) and `~/.bash_profile` load; your own `shell_args` replace that. zsh keeps its history in
    your own `~/.zsh_history` (macOS `/etc/zshrc` pointed it at NOBLE's folder).
  - AI usage finds Claude Code, Codex and Antigravity logins in the Keychain (macOS may ask once whether
    `security` may read Claude Code's).
  - Over SSH into a Mac copied text goes out with OSC 52 and links are copied instead of opened on the Mac's
    screen. The APFS system volumes no longer show the same disk twice, folder names match without regard to
    case, and `file:line` links find VS Code, Cursor, Windsurf or Zed inside `/Applications` without a PATH
    entry.
- Two themes: Ayu Dark and Night Owl.
- `ctrl+click` on a `file:line` path opens it at that line in Cursor, Windsurf or Zed when VS Code is not on
  the PATH (VS Code stays first; without any of them the file opens in the default app as before).
- Until the prefix key has been used once, the status bar tip says how to reach it (`press ctrl+a, then ? for
  all keys`, with your configured prefix).
- README: a short Troubleshooting section (folder tracking, AltGr, colors, clipboard over SSH).

### Changed
- AI usage on Home: cached numbers (`~`) show their age next to the provider name (`~5m`, `~2h`); the error
  line under them no longer repeats it.
- `noble update` run from the notice's tab now says to quit that NOBLE window and start `noble` again.
- bash and zsh panes keep a folder reached through a symlink under its own name (`PWD` is passed on), and the
  outer terminal's `TERMINFO` / iTerm2 variables no longer leak into panes.
- The bash prompt hook works with bash 3.2 and never leaves the shell in the C locale.
- The minimum supported Rust is now 1.95 (`rust-version`). 1.88 was no longer true: `sysinfo` 0.39 already
  needs 1.95 to build.

### Fixed
- Two windows closing at the same moment no longer lose one window's tabs from the saved session: a window
  now waits for another's session lock as long as that window is running (up to 15 s) instead of saving
  without it after 2 s, and takes over a lock left by a window that crashed right away.

### Security
- Opening a file in a terminal editor (ctrl+click on a `file:line` link without a desktop, or the config file
  with `$EDITOR` set) quotes the path for the pane's shell: a file name such as `a$(cmd).rs`, `` a`cmd`.rs ``
  or `a&cmd.rs` could run commands in PowerShell, bash, zsh, fish or cmd. Under cmd a name containing `"`,
  `%` or `!` (which cmd cannot quote safely) is refused with a message.
- On Windows, VS Code (`code.cmd`) and other `.cmd` / `.bat` tools are started so that cmd.exe escapes their
  arguments: ctrl+clicking a `file:line` link whose file name contained `&` or `|` could run commands.
- `noble update` verifies the downloaded archive against the release's SHA-256 checksum before unpacking it.
  A mismatch, or a release without a checksum file (every release up to 1.4.0), stops the update with an error
  and leaves the installed binary untouched; download such a release by hand from the releases page.
- Release archives are published with a `<archive>.sha256` file next to each one (`sha256sum` format), so a
  manual download can be checked too. Updating from 1.4.0 or older still runs the old updater, which does not
  check it; every update from 1.5.0 on is verified.

### Development
- CI checks dependencies with cargo-deny (RustSec advisories, a license allow-list in `deny.toml`, crates.io
  as the only source) on every push, and the advisory check runs weekly as well (`audit.yml`).
- CI builds with the `rust-version` toolchain on Windows and Linux, so the declared minimum stays true.

## [1.4.0]

### Changed
- Settings: sections are regrouped (Display, Terminal, Keys, AI usage, General; the Claude Code hooks moved to
  Terminal). `←` / `→` turn a switch off / on instead of flipping it, `esc` returns to the page Settings was
  opened from, and the wheel scrolls the page with "more" markers when it does not fit. Rows that only matter
  under "Show AI usage" are indented and dimmed while it is off, Quick launch shows that it opens a popup,
  and the duplicate key hint inside the page is gone.
- Settings: the theme grid is now a single Theme row. `⏎` or a click opens a theme selector where the whole
  UI previews the theme under the cursor (`esc` or a click outside reverts), `←` / `→` on the row step
  through the themes directly.
- A terminal tab is named after the folder its focused pane is in now, so a tab opened with prefix `t` (which
  starts in the current folder) gets a new name after a `cd`. Settings → Tab name follows the folder
  (`terminal.tab_follows_cwd = false`) keeps the name of the folder the tab was opened in.
- Terminal tabs have no line on the window's left and right edges: the output uses the full width, and
  lines only run between split panes. Split panes share one border line instead of drawing two side by
  side; the lines meet in `┬ ├ ┤ ┴ ┼`. The focused pane's frame keeps its color all around. A horizontal
  border runs along the lower pane's title row: drag the line to resize, the title and buttons still click.

### Fixed
- Closing the terminal color selector by clicking outside it no longer leaves the previewed scheme on the open
  terminals.
- A setting that cannot be written to `config.toml` now shows an error instead of silently lasting only
  until NOBLE closes.

## [1.3.0]

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
- Several NOBLE windows no longer overwrite each other's session: each window merges its own tabs into the
  session file on exit, and the next launch restores the tabs of every window. Only the first window of a run
  restores; a window opened while another one is running starts empty instead of opening the same tabs again.
- Restored quick-launch tabs (and quick-launch panes in saved workspaces) run their command again, e.g. the AI
  agent starts in its folder. Session files from older versions still load.

### Fixed
- Link detection: URLs inside `( )` or `[ ]`, markdown links `[text](url)`, URLs glued to text
  (`url=https://…`), short URLs such as `http://a` and paths with combining accents are now recognised
  correctly.
- `noble update` never leaves you without a working binary: the new binary is staged next to the old one
  and swapped with renames, an empty binary in the archive is refused, and a leftover `*.old` that cannot
  be deleted no longer blocks the update.
- Better readability: One Dark's dim text and Solarized Light's dim text and accent are slightly darker to
  reach WCAG contrast (3:1 for dim text, 4.5:1 for the accent).
- After Claude exits, its pane and tab no longer keep saying "claude": the agent indicator and labels follow
  the program running now (another agent, or none back at the shell prompt), on Windows and Linux in every
  shell. Claude's `SessionEnd` hook clears the pane's state, and a quick-launch tab is named after its project
  once the launched command has exited.
- Claude hook state left behind by a crashed NOBLE can no longer show up in a new NOBLE that got the same
  process id.
- bash, zsh and fish now percent-encode the directory they report (OSC 7), so a directory whose name ends in a
  space keeps it (spaces, `%`, `;`, `#` and non-ASCII names round-trip exactly).
- A bash started as a login shell (`-l` / `--login` in `shell_args`) lost the cwd tracking: a login shell never
  reads the rc file. NOBLE now loads the login files itself (`/etc/profile`, then `~/.bash_profile`,
  `~/.bash_login` or `~/.profile`) and keeps the tracking.
- No more crashes when selecting text after a lost mouse release during a divider drag, or when a dialog is
  drawn in a tiny window. Editing `config.toml` from Settings keeps a ` #` inside quoted values and headers
  with a trailing comment intact, and a config file that is not valid UTF-8 is reported instead of being
  silently replaced. Corrupt data files with extreme timestamps no longer panic.
- Restoring a session or workspace no longer drops a tab whose folder cannot be entered, and no longer waits
  on a network share that stopped answering (Windows and Linux): after 2 seconds the pane opens in the home
  folder. A notice names the folders that were not available (also for deleted ones).
- Home no longer redraws about 8 times a second forever when a configured AI CLI is not installed or signed
  out: the loading spinner only runs for providers shown on the quota panel (idle Home CPU about 70% lower).

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

[Unreleased]: https://github.com/MertSoylu/noble/compare/v1.9.0...HEAD
[1.9.0]: https://github.com/MertSoylu/noble/compare/v1.8.0...v1.9.0
[1.8.0]: https://github.com/MertSoylu/noble/compare/v1.7.0...v1.8.0
[1.7.0]: https://github.com/MertSoylu/noble/compare/v1.6.0...v1.7.0
[1.6.0]: https://github.com/MertSoylu/noble/compare/v1.5.0...v1.6.0
[1.5.0]: https://github.com/MertSoylu/noble/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/MertSoylu/noble/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/MertSoylu/noble/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/MertSoylu/noble/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/MertSoylu/noble/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/MertSoylu/noble/releases/tag/v1.0.0
