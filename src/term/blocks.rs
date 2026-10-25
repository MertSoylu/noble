//! Command blocks: where each command's output sits in a pane (from the shell's OSC 133 `C` and `D` marks), and
//! the text built from it for the copy and send actions. The marks are read by `term::osc::Callbacks`; this module
//! holds the pure parts (pruning, the context line, the size cap of what goes to an agent).

use std::collections::VecDeque;

/// Most blocks kept per pane; the oldest goes first.
pub const MAX_BLOCKS: usize = 200;

/// What goes to an agent at most: the last lines of the output and their bytes (whichever runs out first).
pub const AGENT_MAX_LINES: usize = 400;
pub const AGENT_MAX_BYTES: usize = 64 * 1024;

/// One command the shell ran, with its output. Line numbers are stable (`pane::screen_base`): output that scrolls
/// away keeps its number, and a reflow (`Screen::reflow_epoch`) renumbers everything, so a block is only good for
/// the epoch it was taken in (see `retain_live`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandBlock {
    /// The first line of the output: the line the cursor was on when the command started running.
    pub start: usize,
    /// The line after the output (exclusive); `None` while the command runs.
    pub end: Option<usize>,
    /// The `reflow_epoch` the line numbers belong to.
    pub epoch: u64,
    /// Exit code from `OSC 133;D;<code>`; `None` when the shell reported none.
    pub exit: Option<i32>,
    /// The line the command was typed on, prompt included, as it showed on screen (`~/app $ cargo test`).
    pub line: Option<String>,
}

/// Keeps only the blocks whose numbers are still valid: the same `epoch`, and a start that is still in the
/// scrollback (`dropped` lines scrolled out before it).
pub fn retain_live(blocks: &mut VecDeque<CommandBlock>, dropped: usize, epoch: u64) {
    blocks.retain(|b| b.epoch == epoch && b.start >= dropped);
}

/// The context line that leads what is sent to an agent: the typed line and the exit code, `→ exit 101`.
/// Without the typed line it says `command`, so the line is never empty.
pub fn context_line(line: Option<&str>, exit: Option<i32>) -> String {
    let typed = line.map(str::trim).filter(|l| !l.is_empty()).unwrap_or("command");
    match exit {
        Some(code) => format!("{typed} → exit {code}"),
        None => typed.to_string(),
    }
}

/// Drops the blank lines at the end of a command's output (the prompt's own empty rows).
pub fn trim_blank_tail(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
}

/// The last lines that fit `max_lines` and `max_bytes` (each line counts with its newline), joined with newlines.
/// Returns the text and how many lines were cut from its start. A single last line longer than `max_bytes` is cut
/// from its start too, at a character boundary.
pub fn capped_output(lines: &[String], max_lines: usize, max_bytes: usize) -> (String, usize) {
    let mut kept = 0;
    let mut bytes = 0;
    for line in lines.iter().rev() {
        if kept == max_lines || (kept > 0 && bytes + line.len() + 1 > max_bytes) {
            break;
        }
        bytes += line.len() + 1;
        kept += 1;
    }
    let cut = lines.len() - kept;
    let mut text = lines[cut..].join("\n");
    if text.len() > max_bytes {
        let mut from = text.len() - max_bytes;
        while !text.is_char_boundary(from) {
            from += 1;
        }
        text = text[from..].to_string();
    }
    (text, cut)
}

/// What is pasted into an agent: the context line (none when `context` is empty), a note when lines were cut, then
/// the output (the newest `AGENT_MAX_LINES` lines within `AGENT_MAX_BYTES`). No line break ends the text: the user
/// presses Enter.
pub fn agent_payload(context: &str, mut lines: Vec<String>) -> String {
    trim_blank_tail(&mut lines);
    let (text, cut) = capped_output(&lines, AGENT_MAX_LINES, AGENT_MAX_BYTES);
    let mut parts = Vec::new();
    if !context.is_empty() {
        parts.push(context.to_string());
    }
    if cut > 0 {
        parts.push(format!("[… {cut} earlier lines cut]"));
    }
    if !text.is_empty() {
        parts.push(text);
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(start: usize, epoch: u64) -> CommandBlock {
        CommandBlock { start, end: Some(start + 1), epoch, exit: Some(0), line: None }
    }

    #[test]
    fn blocks_of_another_epoch_or_a_scrolled_out_start_are_dropped() {
        let mut blocks: VecDeque<CommandBlock> = [block(3, 1), block(10, 2), block(12, 2)].into();
        // A reflow moved the numbering on: every block of the old epoch goes.
        retain_live(&mut blocks, 0, 2);
        assert_eq!(blocks.iter().map(|b| b.start).collect::<Vec<_>>(), vec![10, 12]);
        // Lines scrolled out of history: a block that started before the first kept line goes.
        retain_live(&mut blocks, 11, 2);
        assert_eq!(blocks.iter().map(|b| b.start).collect::<Vec<_>>(), vec![12]);
    }

    #[test]
    fn context_line_leads_with_the_typed_line_and_the_exit_code() {
        assert_eq!(context_line(Some("~/app $ cargo test  "), Some(101)), "~/app $ cargo test → exit 101");
        assert_eq!(context_line(Some("PS C:\\x> ls"), Some(0)), "PS C:\\x> ls → exit 0");
        assert_eq!(context_line(Some("~/app $ ls"), None), "~/app $ ls", "cmd and bash 3.2 report no code");
        assert_eq!(context_line(None, Some(2)), "command → exit 2");
        assert_eq!(context_line(Some("   "), None), "command");
    }

    #[test]
    fn output_cap_keeps_the_newest_lines_within_both_limits() {
        let lines: Vec<String> = (0..10).map(|i| format!("l{i}")).collect();
        let (text, cut) = capped_output(&lines, 3, 1000);
        assert_eq!((text.as_str(), cut), ("l7\nl8\nl9", 7));
        // Bytes: each line costs its length plus a newline, so "l8" and "l9" (3 + 3) fit a limit of 6 only.
        let (text, cut) = capped_output(&lines, 100, 6);
        assert_eq!((text.as_str(), cut), ("l8\nl9", 8));
        let (text, cut) = capped_output(&lines, 100, 1000);
        assert_eq!((text.lines().count(), cut), (10, 0));
    }

    #[test]
    fn one_long_line_is_cut_at_a_character_boundary() {
        let long = "é".repeat(10); // 20 bytes
        let (text, cut) = capped_output(std::slice::from_ref(&long), 400, 7);
        assert_eq!(cut, 0);
        assert!(text.len() <= 7 && long.ends_with(&text), "{text:?}");
    }

    #[test]
    fn agent_payload_has_the_context_first_a_cut_note_and_no_line_break_at_the_end() {
        let lines: Vec<String> = (0..AGENT_MAX_LINES + 5).map(|i| format!("row {i}")).chain(["".to_string()]).collect();
        let text = agent_payload("~/app $ cargo test → exit 101", lines);
        let mut parts = text.splitn(3, '\n');
        assert_eq!(parts.next(), Some("~/app $ cargo test → exit 101"));
        assert_eq!(parts.next(), Some("[… 5 earlier lines cut]"));
        let body = parts.next().unwrap();
        assert!(
            body.starts_with("row 5\n") && body.ends_with(&format!("row {}", AGENT_MAX_LINES + 4)),
            "trailing blank line dropped"
        );
        assert!(!text.ends_with('\n') && !text.ends_with('\r'));
    }

    #[test]
    fn agent_payload_without_output_is_the_context_line_alone() {
        assert_eq!(agent_payload("~/app $ true → exit 0", vec![String::new(), " ".into()]), "~/app $ true → exit 0");
    }
}
