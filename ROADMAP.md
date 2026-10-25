# NOBLE roadmap

The plan for the next stages of NOBLE. The goal: **make NOBLE the best terminal to work with AI coding agents in.**
NOBLE does not run a model of its own; it is the place where agents run. It sees their real state, draws their
screens faithfully, keeps their conversations across restarts and carries context to them. No new panels or
screens: everything lands on the surfaces NOBLE already has (tab dots, pane titles, the Home card, the palette,
Settings).

Within each section the items are in order of priority.

## Starting point

- `cargo clippy --all-targets` is clean and the whole test suite passes (unit, render, PTY, end to end, fuzz).
- v1.10.0 is released.
- Only Claude Code has a real agent state (its hooks). Every other agent is guessed from the program name
  (`agent_kind` in `src/ai/mod.rs`) and shows as a plain `○ codex`.
- The emulator answers only DSR and DA (`unhandled_csi` in `src/term/osc.rs`): synchronized output, color queries,
  focus reports and OSC 9;4 progress are ignored, and a resize cuts lines (no reflow; done in T7).

## 0 · Ship v1.10.0 — done

v1.10.0 shipped as a minor release.

## 1 · Bugs found

- **Key reference overflow (done):** at 160×45 the MOUSE section's key column runs into its text ("double-click
  titlezoom / restore the pane", `target/audit/help-160x45.txt`).
- **Welcome card cut mid-word (done):** the launcher line ends in "… / Antigravit" with no ellipsis at 110, 120 and
  160 columns.
- **Notifications lead nowhere (done):** clicking a "Claude needs you" toast only dismisses it, because `ToastTarget`
  knows only `Config` (`toast_clicked` in `src/app/input.rs`). Add `ToastTarget::Pane` so a click goes to that tab
  and pane; choosing an entry in the notification list should do the same (copying stays as a second choice).
- **Zoom and resize cut lines for good (done):** `vt100` 0.18 `Grid::set_size` truncates the visible rows and does not
  reflow, and zooming resizes the PTY (`App::sync_layout`), so zooming a pane and restoring it loses the end of
  wide lines. Fixed by the reflow in T7.
- **Stale render dumps (done):** `target/audit` keeps dumps of screens that no longer exist. Clear the folder at the
  start of the render tests so the audit only shows current screens.
- **`portable-pty` future incompatibility (watch):** 0.9.0, the latest on crates.io, pulls `nix` 0.28, which a
  future Rust will reject (`cargo report future-incompatibilities`). Track upstream; use `[patch]` if needed. Rust 1.99+ reports nix 0.28 as future-incompatible (warn only; builds with RUSTFLAGS=-D warnings fail); no hard-error date yet (rust-lang/rust#159218 is a draft). Vendor portable-pty when that changes or upstream publishes nix ≥ 0.29.

## 2 · Terminal fidelity: AI CLIs draw in NOBLE as in a native terminal

- **T1 · Synchronized output (done):** `CSI ? 2026 h` / `l` plus the DECRQM answer (`CSI ? 2026 $ p`). Claude Code
  probes for it at startup and uses it only when the terminal reports it; without it NOBLE can draw a half-written
  frame (tearing, flicker). `vt100` passes unknown DEC modes to `unhandled_csi`, so no fork is needed: the reader
  thread holds back `PtyOutput` until the block ends (at most about 150 ms).
- **T2 · Color queries (done):** answer OSC 10/11/12 (and OSC 4) with the pane palette, set `COLORFGBG` in panes and
  support mode 2031 (color scheme change notices). Claude Code's `auto` theme, neovim and bat/delta pick light or
  dark from OSC 11: the right colors on the light themes (Latte, Paper, Solarized Light, light-gray), and a live
  update when the NOBLE theme changes.
- **T3 · Focus reports (done):** DECSET 1004, `CSI I` / `CSI O` when the pane or the window gains or loses the focus
  (vim's autoread, tools that notify only while in the background).
- **T4 · Identity and mode queries (done):** XTVERSION (`CSI > q` → `NOBLE 1.x`) and DECRQM for the modes NOBLE
  knows, so programs can recognize it.
- **T5 · OSC 9;4 progress:** ignored today (`src/term/osc.rs`); show it in the pane title and on the tab.
  Claude Code already sends it, and it gives agents without hooks a generic "busy" signal.
- **T6 · Claude Code's fullscreen renderer:** now the default for new Claude Code users. Check the wheel,
  clicks, shift+drag selection and ctrl+click inside NOBLE on Windows, Linux and macOS. Claude's docs warn that
  ConPTY-based hosts can leave stale fragments on screen, and NOBLE's panes run on ConPTY on Windows: add a render or
  PTY test, and a documented fix if needed.
- **T7 · Reflow (done):** `vt100` 0.18.1 is vendored as `src/vt100` and reflows the primary screen and its
  scrollback on a width change; search, OSC 8 links and the scrollback bar use stable line numbers
  (`history_dropped`, reflow epoch). Spike options were: (a) a `vt100` fork that reflows using the rows' wrap flags, or (b) moving to
  `alacritty_terminal` (reflow, the kitty keyboard protocol and synchronized output built in, but `term/osc.rs` and
  `ui/terminal.rs` rewritten). The decision and a prototype; the implementation comes later.

## 3 · AI integration: NOBLE hosts agents, it does not run one

### A1 · Real state for every agent (done; Antigravity and Cursor later)

Other agent CLIs now have hook systems like Claude Code's:

- **Codex (done):** `~/.codex/hooks.json`, with `UserPromptSubmit`, `PermissionRequest`, `Stop`, `Interrupt`,
  `SessionEnd` and `SubagentStart` / `SubagentStop`. User-level hooks also need a one-time trust in Codex
  (`/hooks`): accepted, NOBLE says so once after installing them.
- **Copilot CLI (done):** `~/.copilot/hooks/` (`$COPILOT_HOME/hooks/` when set). NOBLE writes its own `noble.json` there,
  so removing the integration is deleting one file. Events: `userPromptSubmitted`, `agentStop`, `notification`
  (`permission_prompt`, `elicitation_dialog`).
- **OpenCode (done):** a plugin at `~/.config/opencode/plugins/noble.js` (`$XDG_CONFIG_HOME/opencode` when set) that
  passes session status and idle, permission requests and session errors to `noble hook --agent opencode`.
- **Antigravity (`agy`), later:** hooks in `~/.gemini/settings.json` (event names to be verified). Cursor CLI's hook
  support is partial: later.

Design (done): one module per agent in `src/hooks/` (installed or not, install / uninstall / upgrade, event → state
map); `noble hook <event> --agent codex`; `HookRecord` gets an `agent` field (records without one are Claude's).
The Settings row "Claude Code status hooks" became a popup like Quick launch ("Agent status hooks", one row per
agent found here), and the one-time offer covers every agent. Tests: install / uninstall / upgrade per agent,
idempotent and keeping the user's own hooks; the e2e tests stay isolated (`NOBLE_NO_SYSTEM_INTEGRATIONS`).

Result: the tab dots, "needs you", prefix `a`, the notifications and the session list on Home show the real state
of every agent. No new UI.

### A2 · Finer Claude states (done)

- `StopFailure` (an API error, a rate limit) → a `✗ claude · error` state and a notification.
- `PermissionRequest` → "needs you" at once.
- Clearer notification texts from `notification_type` and the tool name ("asks to run Bash", "asks a question").

### A3 · Conversations come back

A restored quick-launch tab starts `claude` again today: a new, empty conversation.

- The session id the hooks receive on stdin is kept in the record, saved with the pane (`SavedNode::Leaf` gets a
  `resume` field) and restored as `claude --resume <id>`, `codex resume <id>` or `copilot --resume <id>`.
- The same for workspaces and prefix `T` (reopen closed tab). Setting: "Resume agent conversations" (on by
  default, to be confirmed).
- One line on the Home card: `↻ claude · 2h ago` runs `claude --continue` in the project. Only the modification time
  of the project's folder under `~/.claude/projects/` is read, never a transcript.

### A4 · Carry context to the agent

- The emulator keeps the absolute lines of the OSC 133 C and D marks, so every command's output range is known
  ("command blocks").
- After a failing test run, "Send Last Output to Agent" (pane menu, palette, a prefix key) pastes it into the agent
  pane of the same tab (else the last one used) with bracketed paste, led by one context line
  (`~/app $ cargo test → exit 101`). **It never presses Enter**: you read it and send it. A selection, when there
  is one, is sent instead.
- The same marks give the terminal more: jump to the previous / next command, copy the last output, and a click on
  a `✗` badge scrolls to the failed command.

### A5 · Smarter quota (no new network calls)

- When a window that passed `warn_at` resets: "Claude 5h quota reset" (from the cached `resets_at`).
- A full quota shows `⚠` on its launcher chip, and launching says when it resets ("resets in 1h12m").
- An agent pane's title shows `⚠ 5h 92%` only past `warn_at`, so the warning is where you work.
- Optional: GitHub Copilot's premium request quota (`api.github.com/copilot_internal/user`, an undocumented
  endpoint: verify first; any failure is a silent `None`).

### A6 · Awareness while NOBLE is in the background

- While an agent waits for you, NOBLE sends OSC 9;4 "paused" to the terminal it runs in: Windows Terminal shows it
  on the tab and the taskbar icon, Ghostty as a progress bar, and terminals that do not know it ignore it. It is
  cleared when nothing waits.
- The window title follows the state (`NOBLE · ◆ 1`); today it is fixed (`SetTitle` in `src/main.rs`).

### A7 · Background Claude sessions (optional, to be decided)

Sessions started with `claude --bg` listed with the existing session rows on Home (`claude agents --json --all`,
only while Home is open, at most once a minute); a click attaches to one in a new tab (`claude attach <id>`).

## 4 · UX and interface

- **A shorter pane menu:** every installed agent adds its own "Launch X in split" row (`src/app/menu.rs`), 19
  rows in the render test. One "Launch Agent in Split ›" sub-list instead.
- **Missing pane operations:** Equalize Splits, Swap With Next Pane and Move Pane to New Tab, as actions with
  pane menu and palette entries (no new UI).
- **Home card:** use the empty space on tall screens: the number of commits follows the height (`MAX_COMMITS`
  is a fixed 8 in `src/projects.rs`), plus the `↻` resume line from A3.
- **`noble doctor` (done):** version, OS, outer terminal and truecolor, shells and their integration, agent hooks, AI
  provider detection (never a token), config and data paths, `gh` state. The bug report template asks for its
  output.

## 5 · Distribution

Installing is the biggest barrier for new users:

- `cargo binstall noble` (`[package.metadata.binstall]`, mapping the release archive names).
- A Scoop bucket and a winget manifest (Windows), a Homebrew tap (macOS and Linux; it also avoids the macOS
  quarantine step), kept up to date by `release.yml`.
- An "AI agents" page in the docs: hooks per agent, resume, send to agent.

## Plan

1. **Stage 1**
   - Focus: v1.10.0, section 1 bugs, T1–T4, notification → pane, `noble doctor`
   - Release: v1.10.0, v1.10.1
2. **Stage 2 (done)**
   - Focus: A1 (Codex and Copilot first, then OpenCode), A2
   - Release: v1.11.0
3. **Stage 3**
   - Focus: A3 resume, A4 command blocks and send to agent
   - Release: v1.12.0
4. **Stage 4**
   - Focus: A5, A6, T5–T6, pane operations and menu, T7 spike, packaging
   - Release: v1.12.x
5. **Stage 5**
   - Focus: Docs, website, screenshots
   - Release: —

Every release: CI green on Windows, Linux and macOS; the idle cost
(`cargo test --release --test e2e idle -- --ignored --nocapture`) no worse than the README's figures; the
`target/audit` dumps reviewed.

**Done when:**

- Claude Code, Codex, Copilot and OpenCode show their real state, with prefix `a` and notifications.
- After a restart the agents' conversations continue where they left off.
- Claude Code draws without tearing in NOBLE, with the right colors on light themes.
- Installing is one command on every platform.

## Out of scope

A separate agent panel or overlay (agent animations, permission cards, output previews), a diff viewer, a browser
tab, a worktree manager, an in-app LLM or chat. The kitty keyboard protocol, images (sixel, kitty graphics) and the
reflow implementation wait for a later stage.

## Open decisions

1. Resume agent conversations on by default?
2. A7 (background Claude sessions on Home): yes or no?
3. Which package managers: winget, Scoop, Homebrew, AUR?

## References

- [Codex hooks](https://learn.chatgpt.com/docs/hooks.md)
- [Copilot CLI hooks reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-hooks-reference)
- [Copilot CLI session data and `--resume`](https://docs.github.com/en/copilot/how-tos/copilot-cli/use-copilot-cli/chronicle)
- [OpenCode plugins](https://opencode.ai/docs/plugins/)
- [Gemini CLI hooks](https://geminicli.com/docs/hooks.md)
- [Agent hooks in Antigravity CLI](https://medium.com/google-cloud/a-developers-guide-to-agent-hooks-in-antigravity-cli-4c1440febd11)
- [Cursor CLI hooks (forum)](https://forum.cursor.com/t/cursor-cli-hooks/148511)
- [Claude Code hooks reference](https://code.claude.com/docs/en/hooks)
- [Claude Code fullscreen rendering](https://code.claude.com/docs/en/fullscreen)
- [Claude Code agent view](https://code.claude.com/docs/en/agent-view)
- [Claude Code `auto` theme and OSC 11 (issue)](https://claudeissues.com/issue/56848-theme-auto-osc-11-detects-light-terminal-but-syntax-highlighted-code-still-rende)
- [Copilot quota endpoint (third-party notes)](https://lzw.me/docs/opencodedocs/vbgate/opencode-mystatus/appendix/api-endpoints/)
