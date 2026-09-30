//! Notifications forwarded to the terminal NOBLE itself runs in, so a background tab that needs you (or a long
//! command that finished) can raise a desktop notification while the NOBLE window is not in front.

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
}
