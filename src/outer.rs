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

/// Whether the OSC 9 / OSC 777 pair should go to the outer terminal.
///
/// - Linux/macOS: always. iTerm2, WezTerm, Ghostty, kitty (OSC 9) and foot, VTE-based terminals with the
///   notification extension (OSC 777) show it; terminals that do not know the sequences ignore them.
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

/// The escape sequences for one notification: OSC 9 and OSC 777, both ended by BEL. Empty when the text is empty.
///
/// The OSC 9 text starts with a fixed word: OSC 9 followed by a number is a ConEmu command (`9;4` progress,
/// `9;9` cwd …) and a message that happens to begin with a digit must not be read as one.
pub fn sequences(text: &str) -> String {
    let text = sanitize(text);
    if text.is_empty() {
        return String::new();
    }
    format!("\x1b]9;NOBLE: {text}\x07\x1b]777;notify;NOBLE;{text}\x07")
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
    fn sequences_are_osc_9_and_777() {
        let s = sequences("tab · done in 12s");
        assert_eq!(s, "\x1b]9;NOBLE: tab · done in 12s\x07\x1b]777;notify;NOBLE;tab · done in 12s\x07");
        // A leading number cannot turn into a ConEmu command.
        assert!(sequences("4;1;50").starts_with("\x1b]9;NOBLE: 4,1,50"));
        assert_eq!(sequences("\x07"), "");
        // Injected escape sequences stay inert: exactly two terminators.
        assert_eq!(sequences("x\x07\x1b[31m").matches('\x07').count(), 2);
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
