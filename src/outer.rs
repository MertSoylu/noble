//! Notifications forwarded to the terminal NOBLE itself runs in, so a background tab that needs you (or a long
//! command that finished) can raise a desktop notification while the NOBLE window is not in front. The same
//! terminal shows the agents waiting for you: a paused progress report and the window title.

/// Longest notification text sent out, in characters.
const MAX_CHARS: usize = 160;

/// Text that is safe inside an OSC string: control characters (ESC, BEL, newlines …) are dropped, the `;` field
/// separator becomes `,`, whitespace is squeezed and the length is capped.
pub fn sanitize(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .map(|c| if c == ';' { ',' } else { c })
        .collect();
    let squeezed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    squeezed.chars().take(MAX_CHARS).collect()
}

/// Whether a notification goes to the outer terminal at all (`notify_form` picks the sequence).
///
/// - Linux/macOS: always. Terminals that do not know the sequences ignore them.
/// - Windows: only when `TERM_PROGRAM` is set (WezTerm, VS Code and the like, which understand them). Windows
///   Terminal shows nothing for these sequences (its OSC 9 is only the `9;4` progress and `9;9` cwd forms) and
///   the legacy console host has no use for them, so there the BEL that NOBLE sends anyway (taskbar flash,
///   tab marker) is the whole signal.
pub fn supported(windows: bool, term_program_set: bool) -> bool {
    !windows || term_program_set
}

/// Whether the OSC 9;4 progress report (the paused state, see `changes`) should go to the outer terminal.
///
/// - Linux/macOS: always. Terminals that do not know OSC 9;4 ignore it.
/// - Windows: only when `WT_SESSION` (Windows Terminal, which shows it on the tab and the taskbar icon) or
///   `TERM_PROGRAM` is set. The legacy console host has no use for it; there the window title is the whole signal.
pub fn progress_supported(windows: bool, term_program_set: bool, wt_session_set: bool) -> bool {
    !windows || term_program_set || wt_session_set
}

/// The sequence a terminal shows as a desktop notification. NOBLE sends exactly one: a terminal that knows both
/// (WezTerm, Ghostty, foot) would show every notification twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyForm {
    /// `OSC 9 ; text` (iTerm2's form). Also the default for a terminal NOBLE does not know.
    Osc9,
    /// `OSC 777 ; notify ; title ; text` (rxvt's form).
    Osc777,
}

/// The notification form for the terminal NOBLE runs in, from the environment it was started in: `TERM_PROGRAM`,
/// `TERM`, whether `KITTY_WINDOW_ID` is set and whether `KONSOLE_VERSION` is.
///
/// - OSC 9: iTerm2 (`TERM_PROGRAM=iTerm.app`), WezTerm, Ghostty, and kitty (`TERM=xterm-kitty` or
///   `KITTY_WINDOW_ID`; kitty reads OSC 9 as its legacy form, its own protocol is OSC 99, which NOBLE does not send).
/// - OSC 777: foot (`TERM=foot…`), rxvt-unicode (`TERM=rxvt…`) and Konsole, which shows OSC 777 and not OSC 9.
/// - Otherwise OSC 9. GNOME Terminal (VTE) shows neither sequence upstream, so the choice makes no difference there.
pub fn notify_form(term_program: Option<&str>, term: Option<&str>, kitty_window: bool, konsole: bool) -> NotifyForm {
    let term = term.unwrap_or("");
    let osc9 =
        matches!(term_program, Some("iTerm.app" | "WezTerm" | "ghostty")) || term == "xterm-kitty" || kitty_window;
    let osc777 = konsole || term.starts_with("foot") || term.starts_with("rxvt");
    if osc777 && !osc9 { NotifyForm::Osc777 } else { NotifyForm::Osc9 }
}

/// The escape sequence for one notification in `form` (see `notify_form`), ended by BEL. Empty when the text is
/// empty.
///
/// The OSC 9 text starts with a fixed word: OSC 9 followed by a number is a ConEmu command (`9;4` progress,
/// `9;9` cwd …) and a message that happens to begin with a digit must not be read as one.
pub fn sequences(text: &str, form: NotifyForm) -> String {
    let text = sanitize(text);
    if text.is_empty() {
        return String::new();
    }
    match form {
        NotifyForm::Osc9 => format!("\x1b]9;NOBLE: {text}\x07"),
        NotifyForm::Osc777 => format!("\x1b]777;notify;NOBLE;{text}\x07"),
    }
}

/// OSC 9;4 state 4 is the paused report (Windows Terminal's warning, Ghostty's pause); state 0 clears it. Both are
/// ended by ST (ESC \); the notifications and the title in this file end with BEL.
const PAUSED: &str = "\x1b]9;4;4;0\x1b\\";
const CLEARED: &str = "\x1b]9;4;0;0\x1b\\";

/// Pushes the window title on the terminal's title stack (XTWINOPS 22) before NOBLE sets its own; `RESTORE_TITLE`
/// on exit puts the old one back. Terminals without the stack ignore both.
pub const SAVE_TITLE: &str = "\x1b[22;0t";
pub const RESTORE_TITLE: &str = "\x1b[23;0t";

/// What NOBLE shows on the outer terminal: the paused progress report and the window title.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Some agent waits for the user.
    pub paused: bool,
    /// `NOBLE` or `NOBLE dev`, with `· ◆ N` while N agents wait. Empty until the first write.
    pub title: String,
}

impl Status {
    /// The status with `waiting` agents that need the user, in the dev build (`dev`) or the stable one.
    pub fn new(dev: bool, waiting: usize) -> Status {
        let name = if dev { "NOBLE dev" } else { "NOBLE" };
        let title = if waiting == 0 { name.to_string() } else { format!("{name} · ◆ {waiting}") };
        Status { paused: waiting > 0, title }
    }
}

/// The sequences that move the outer terminal from `shown` to `next`, for what changed only. The progress report is
/// written only where `progress` is supported (`progress_supported`). The title comes from `Status::new`, so it never
/// carries text the terminal sent.
pub fn changes(shown: &Status, next: &Status, progress: bool) -> String {
    let mut out = String::new();
    if progress && shown.paused != next.paused {
        out.push_str(if next.paused { PAUSED } else { CLEARED });
    }
    if shown.title != next.title {
        out.push_str(&format!("\x1b]0;{}\x07", next.title));
    }
    out
}

/// The sequences NOBLE leaves behind on exit: the progress report cleared (where it is used) and the title restored.
pub fn exit_sequences(progress: bool) -> String {
    let clear = if progress { CLEARED } else { "" };
    format!("{clear}{RESTORE_TITLE}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_controls_and_caps_length() {
        assert_eq!(sanitize("a\x1b]0;evil\x07b\nc; d"), "a]0,evilb c, d");
        assert_eq!(sanitize("  spaced   out  "), "spaced out");
        assert_eq!(sanitize(&"x".repeat(500)).chars().count(), MAX_CHARS);
        assert_eq!(sanitize("\x1b\x07"), "");
    }

    #[test]
    fn sequences_send_only_the_chosen_form() {
        assert_eq!(sequences("tab · done in 12s", NotifyForm::Osc9), "\x1b]9;NOBLE: tab · done in 12s\x07");
        assert_eq!(sequences("tab · done in 12s", NotifyForm::Osc777), "\x1b]777;notify;NOBLE;tab · done in 12s\x07");
        // A leading number cannot turn into a ConEmu command.
        assert!(sequences("4;1;50", NotifyForm::Osc9).starts_with("\x1b]9;NOBLE: 4,1,50"));
        for form in [NotifyForm::Osc9, NotifyForm::Osc777] {
            assert_eq!(sequences("\x07", form), "");
            // Injected escape sequences stay inert: exactly one terminator, and no second notification.
            let out = sequences("x\x07\x1b[31m", form);
            assert_eq!(out.matches('\x07').count(), 1);
            assert_eq!(out.matches("\x1b]").count(), 1);
        }
    }

    #[test]
    fn notify_form_picks_one_sequence_per_terminal() {
        // OSC 9 terminals.
        assert_eq!(notify_form(Some("iTerm.app"), None, false, false), NotifyForm::Osc9);
        assert_eq!(notify_form(Some("WezTerm"), Some("xterm-256color"), false, false), NotifyForm::Osc9);
        assert_eq!(notify_form(Some("ghostty"), Some("xterm-ghostty"), false, false), NotifyForm::Osc9);
        assert_eq!(notify_form(None, Some("xterm-kitty"), false, false), NotifyForm::Osc9);
        assert_eq!(notify_form(None, Some("xterm-256color"), true, false), NotifyForm::Osc9, "kitty by its window id");
        // OSC 777 terminals.
        assert_eq!(notify_form(None, Some("foot"), false, false), NotifyForm::Osc777);
        assert_eq!(notify_form(None, Some("foot-extra"), false, false), NotifyForm::Osc777);
        assert_eq!(notify_form(None, Some("rxvt-unicode-256color"), false, false), NotifyForm::Osc777);
        assert_eq!(notify_form(None, Some("xterm-256color"), false, true), NotifyForm::Osc777, "Konsole");
        // A terminal that matches both keeps OSC 9.
        assert_eq!(notify_form(None, Some("xterm-kitty"), false, true), NotifyForm::Osc9);
        // Anything else, and no environment at all, gets OSC 9.
        assert_eq!(notify_form(Some("vscode"), Some("xterm-256color"), false, false), NotifyForm::Osc9);
        assert_eq!(notify_form(Some("tmux"), Some("tmux-256color"), false, false), NotifyForm::Osc9);
        assert_eq!(notify_form(None, None, false, false), NotifyForm::Osc9);
    }

    #[test]
    fn windows_needs_a_terminal_that_understands_them() {
        assert!(supported(false, false));
        assert!(supported(true, true));
        assert!(!supported(true, false));
    }

    #[test]
    fn progress_needs_a_terminal_that_shows_it_on_windows() {
        assert!(progress_supported(false, false, false));
        assert!(progress_supported(true, false, true), "Windows Terminal");
        assert!(progress_supported(true, true, false), "WezTerm, VS Code and the like");
        assert!(!progress_supported(true, false, false), "the legacy console host");
    }

    #[test]
    fn status_names_the_waiting_agents_in_the_title() {
        assert_eq!(Status::new(false, 0), Status { paused: false, title: "NOBLE".into() });
        assert_eq!(Status::new(false, 1), Status { paused: true, title: "NOBLE · ◆ 1".into() });
        assert_eq!(Status::new(true, 3), Status { paused: true, title: "NOBLE dev · ◆ 3".into() });
    }

    #[test]
    fn changes_write_only_what_moved() {
        let idle = Status::new(false, 0);
        let waiting = Status::new(false, 1);
        // The first write sets the title (the default status has none yet); nothing waits, so no pause.
        assert_eq!(changes(&Status::default(), &idle, true), "\x1b]0;NOBLE\x07");
        assert_eq!(changes(&idle, &idle, true), "");
        assert_eq!(changes(&idle, &waiting, true), "\x1b]9;4;4;0\x1b\\\x1b]0;NOBLE · ◆ 1\x07");
        // Another agent waiting moves the count; the pause is already shown and not repeated.
        assert_eq!(changes(&waiting, &Status::new(false, 2), true), "\x1b]0;NOBLE · ◆ 2\x07");
        assert_eq!(changes(&waiting, &idle, true), "\x1b]9;4;0;0\x1b\\\x1b]0;NOBLE\x07");
        // Without the progress report only the title is written.
        assert_eq!(changes(&idle, &waiting, false), "\x1b]0;NOBLE · ◆ 1\x07");
    }

    #[test]
    fn exit_clears_the_report_and_restores_the_title() {
        assert_eq!(exit_sequences(true), "\x1b]9;4;0;0\x1b\\\x1b[23;0t");
        assert_eq!(exit_sequences(false), "\x1b[23;0t");
    }
}
