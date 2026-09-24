//! System clipboard. One `arboard::Clipboard` lives for the whole run: on X11 the
//! copied text is served by the process that owns it, so a handle dropped right
//! after copying could lose the text when no clipboard manager takes it over.
//! Without a system clipboard (SSH, a Linux console, no X11/Wayland) copied text
//! goes to the outer terminal with OSC 52, which most terminals put on the
//! clipboard of the machine the user sits at.

use std::io::Write;

/// How the copied text left NOBLE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copied {
    System,
    /// Sent to the outer terminal (OSC 52).
    Terminal,
}

/// Longest text sent with OSC 52 (many terminals drop larger requests).
const OSC52_MAX: usize = 100_000;

pub struct Clipboard {
    inner: Option<arboard::Clipboard>,
    /// Whether OSC 52 may be written to stdout (off in headless tests).
    osc52: bool,
}

impl Clipboard {
    pub fn new(osc52: bool) -> Clipboard {
        Clipboard { inner: None, osc52 }
    }

    /// The system clipboard, opened on first use (and again after a failure).
    fn system(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.inner.is_none() {
            self.inner = arboard::Clipboard::new().ok();
        }
        self.inner.as_mut()
    }

    pub fn set(&mut self, text: &str) -> Option<Copied> {
        if let Some(c) = self.system() {
            if c.set_text(text.to_string()).is_ok() {
                return Some(Copied::System);
            }
            // The connection may be stale (e.g. the X server restarted): retry once with a new one.
            self.inner = None;
            if self.system().is_some_and(|c| c.set_text(text.to_string()).is_ok()) {
                return Some(Copied::System);
            }
        }
        (self.osc52 && text.len() <= OSC52_MAX && write_osc52(text).is_ok()).then_some(Copied::Terminal)
    }

    pub fn get(&mut self) -> Option<String> {
        self.system()?.get_text().ok()
    }
}

fn write_osc52(text: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    write!(out, "\x1b]52;c;{}\x07", crate::util::base64_encode(text.as_bytes()))?;
    out.flush()
}
