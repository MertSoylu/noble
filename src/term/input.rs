//! Turns keyboard and mouse events into xterm byte sequences sent to the PTY.

use crate::vt100::{MouseProtocolEncoding, MouseProtocolMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

/// xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
fn modifier_param(mods: KeyModifiers) -> u8 {
    let mut m = 1;
    if mods.contains(KeyModifiers::SHIFT) {
        m += 1;
    }
    if mods.contains(KeyModifiers::ALT) {
        m += 2;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        m += 4;
    }
    m
}

/// A non-letter character arriving with Ctrl+Alt was produced by AltGr, and so is a letter the keyboard layout
/// types with AltGr (Turkish Q: AltGr+I is "i").
pub fn is_altgr_char(mods: KeyModifiers, c: char) -> bool {
    mods.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) && (!c.is_ascii_alphabetic() || layout_altgr_letter(c))
}

/// Windows: whether the keyboard layout types `c` with AltGr. ConPTY turns text it is given (SSH, a paste without
/// bracketed paste, NOBLE inside NOBLE) into key presses the same way, so on Turkish Q a plain "i" arrives as
/// Ctrl+Alt+I. Linux and macOS terminals send the character AltGr/Option produced, never Ctrl+Alt: always false.
fn layout_altgr_letter(c: char) -> bool {
    #[cfg(windows)]
    {
        let mut utf16 = [0u16; 2];
        let &mut [unit] = c.encode_utf16(&mut utf16) else { return false };
        // SAFETY: VkKeyScanW only reads its argument.
        let scan = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW(unit) };
        // Low byte: the virtual key, high byte: the shift state (1 shift, 2 ctrl, 4 alt); -1 when no key types it.
        let shift = (scan as u16 >> 8) as u8;
        scan != -1 && shift & 6 == 6
    }
    #[cfg(not(windows))]
    {
        let _ = c;
        false
    }
}

/// Kitty keyboard protocol flag 1, "disambiguate escape codes": the one flag NOBLE supports. Other flags a program
/// pushes are masked off (`term::osc::Callbacks`), so a query reports exactly what NOBLE does.
pub const KITTY_DISAMBIGUATE: u8 = 1;

/// How a pane's program wants its keys encoded, read from the emulator (`pane::key_mode`). `Default` is the plain
/// xterm encoding that a program gets when it asked for nothing.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct KeyMode {
    /// DECCKM (application cursor keys, `CSI ? 1 h`): the cursor keys send `SS3` instead of `CSI`.
    pub app_cursor: bool,
    /// Kitty keyboard protocol flags of the screen that is shown (`CSI > f u`, see `Callbacks::kitty_flags`).
    pub kitty: u8,
    /// The program asked for the kitty flags (`CSI ? u`) and has not pushed or set any since (see
    /// `Callbacks::kitty_asked`). Only the Shift+Enter fallback in `encode_key` reads it.
    pub kitty_asked: bool,
    /// win32-input-mode is on (`CSI ? 9001 h`, see `Callbacks::win32_input`): the keys `win32_key` covers go as
    /// console input records.
    pub win32: bool,
}

/// Kitty's modifier parameter: the xterm one plus Super (8), Hyper (16) and Meta (32).
fn kitty_param(mods: KeyModifiers) -> u8 {
    modifier_param(mods)
        + if mods.contains(KeyModifiers::SUPER) { 8 } else { 0 }
        + if mods.contains(KeyModifiers::HYPER) { 16 } else { 0 }
        + if mods.contains(KeyModifiers::META) { 32 } else { 0 }
}

/// Keys under the kitty "disambiguate" flag: the ones whose legacy bytes are ambiguous (Esc, Ctrl+c, Shift+Enter)
/// are sent as `CSI <code> ; <m> u`. `None` keeps the legacy bytes: a plain text key, and every key that already has
/// its own unambiguous sequence (arrows, editing keys, function keys).
fn disambiguated(code: KeyCode, mods: KeyModifiers) -> Option<Vec<u8>> {
    let csi_u = |code: u32, m: u8| -> Vec<u8> {
        if m == 1 { format!("\x1b[{code}u") } else { format!("\x1b[{code};{m}u") }.into_bytes()
    };
    let m = kitty_param(mods);
    let modified = m > 1;
    match code {
        KeyCode::Esc => Some(csi_u(27, m)),
        KeyCode::Enter if modified => Some(csi_u(13, m)),
        KeyCode::Tab if modified => Some(csi_u(9, m)),
        // Shift+Tab is the back tab whether or not crossterm reports the Shift.
        KeyCode::BackTab => Some(csi_u(9, kitty_param(mods | KeyModifiers::SHIFT))),
        KeyCode::Backspace if modified => Some(csi_u(127, m)),
        // A letter is sent in lower case (Ctrl+Shift+A is `CSI 97 ; 6 u`); other characters as they are. With the
        // alternate-keys flag crossterm moves Shift into the letter (Ctrl+Shift+C is `Char('C')` with CONTROL), so a
        // capital here always had Shift typed on it.
        KeyCode::Char(c)
            if mods.intersects(
                KeyModifiers::CONTROL
                    | KeyModifiers::ALT
                    | KeyModifiers::SUPER
                    | KeyModifiers::HYPER
                    | KeyModifiers::META,
            ) =>
        {
            if c.is_ascii_uppercase() {
                Some(csi_u(u32::from(c.to_ascii_lowercase()), kitty_param(mods | KeyModifiers::SHIFT)))
            } else {
                Some(csi_u(u32::from(c), m))
            }
        }
        _ => None,
    }
}

/// win32-input-mode (`CSI ? 9001 h`): a key goes as the console input record Windows would make for it, once for the
/// key down (`Kd` 1) and once for the key up (`Kd` 0): `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`. Only the keys whose legacy
/// bytes lose their modifier are sent so: Enter with Shift and/or Ctrl (Alt may be added), Esc with any modifiers, Tab
/// and Backspace with Ctrl (Shift may be added). Every other key keeps its legacy bytes. ConPTY turns the mode on at
/// startup for the programs that read console input records (PSReadLine, `[Console]::ReadKey`). The encoding is not
/// gated on Windows: whoever asks for the mode gets it, and on Unix nothing does.
fn win32_key(code: KeyCode, mods: KeyModifiers) -> Option<Vec<u8>> {
    const SHIFT_PRESSED: u32 = 0x10;
    const LEFT_CTRL_PRESSED: u32 = 0x08;
    const LEFT_ALT_PRESSED: u32 = 0x02;
    // crossterm reports Shift+Tab as the back tab, so the back tab carries Shift whether or not it is in `mods`.
    let shift = mods.contains(KeyModifiers::SHIFT) || code == KeyCode::BackTab;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    // Virtual key, scan code and the character (Enter with Ctrl is LF, Backspace with Ctrl is DEL).
    let (vk, sc, uc): (u32, u32, u32) = match code {
        KeyCode::Enter if shift || ctrl => (13, 28, if ctrl { 10 } else { 13 }),
        KeyCode::Esc => (27, 1, 27),
        KeyCode::Tab | KeyCode::BackTab if ctrl && !alt => (9, 15, 9),
        KeyCode::Backspace if ctrl && !alt => (8, 14, 127),
        _ => return None,
    };
    let mut cs = 0;
    if shift {
        cs |= SHIFT_PRESSED;
    }
    if ctrl {
        cs |= LEFT_CTRL_PRESSED;
    }
    if alt {
        cs |= LEFT_ALT_PRESSED;
    }
    let record = |key_down: u8| format!("\x1b[{vk};{sc};{uc};{key_down};{cs};1_");
    Some(format!("{}{}", record(1), record(0)).into_bytes())
}

/// Encodes a key event into bytes. `mode` says what the program asked for (see `KeyMode`); `KeyMode::default()`
/// is the plain encoding. Kitty (`mode.kitty`) comes first: a program that pushed its flags gets CSI u, and ConPTY
/// passes CSI u through to programs that read VT input. Then the Shift+Enter fallback (`kitty_asked`), then
/// win32-input-mode for the keys `win32_key` covers.
pub fn encode_key(ev: &KeyEvent, mode: KeyMode) -> Vec<u8> {
    let app_cursor = mode.app_cursor;
    let mut mods = ev.modifiers;
    // Windows AltGr = Ctrl+Alt: produced characters like '@', '{', '|', '€'
    // are sent as they are and never turned into control codes.
    if let KeyCode::Char(c) = ev.code
        && is_altgr_char(mods, c)
    {
        mods = KeyModifiers::NONE;
    }
    if mode.kitty & KITTY_DISAMBIGUATE != 0
        && let Some(bytes) = disambiguated(ev.code, mods)
    {
        return bytes;
    }
    // A program that asked for the kitty flags but whose answer was lost still parses CSI u, and a plain `\r` would
    // submit its prompt (Claude Code's Shift+Enter on a rare start). Only Shift alone is changed: Ctrl+Enter stays on
    // the legacy path, and without an ask the bytes stay as they were.
    if mode.kitty & KITTY_DISAMBIGUATE == 0
        && mode.kitty_asked
        && ev.code == KeyCode::Enter
        && mods == KeyModifiers::SHIFT
    {
        return b"\x1b[13;2u".to_vec();
    }
    if mode.win32
        && let Some(bytes) = win32_key(ev.code, mods)
    {
        return bytes;
    }
    // Legacy input has no bytes for Super, Hyper and Meta: without the kitty form above, such a key sends nothing
    // (the bare letter would be typed into the shell).
    if mods.intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META) {
        return Vec::new();
    }
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let plain = !mods.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL);
    let m = modifier_param(mods);

    let with_alt = |mut bytes: Vec<u8>| {
        if alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    // CSI cursor keys: ESC [ x / ESC O x / ESC [ 1 ; m x
    let cursor = |letter: u8| -> Vec<u8> {
        if plain {
            if app_cursor { vec![0x1b, b'O', letter] } else { vec![0x1b, b'[', letter] }
        } else {
            format!("\x1b[1;{m}{}", letter as char).into_bytes()
        }
    };
    // Tilde keys: ESC [ n ~ / ESC [ n ; m ~
    let tilde = |n: u8| -> Vec<u8> {
        if plain { format!("\x1b[{n}~").into_bytes() } else { format!("\x1b[{n};{m}~").into_bytes() }
    };

    match ev.code {
        KeyCode::Char(c) => {
            if ctrl {
                let lower = c.to_ascii_lowercase();
                let byte = match lower {
                    'a'..='z' => Some(lower as u8 - b'a' + 1),
                    ' ' | '@' | '2' => Some(0),
                    '[' | '3' => Some(0x1b),
                    '\\' | '4' => Some(0x1c),
                    ']' | '5' => Some(0x1d),
                    '^' | '6' => Some(0x1e),
                    // Ctrl+/ is `/` in the kitty form and `7` in legacy input (crossterm reads 0x1f as Ctrl+7).
                    '_' | '-' | '/' | '7' => Some(0x1f),
                    '?' | '8' => Some(0x7f),
                    _ => None,
                };
                if let Some(b) = byte {
                    return with_alt(vec![b]);
                }
            }
            // Alt+Shift+x comes from the kitty protocol as the lower-case letter with SHIFT (`CSI 120 ; 4 u`), but
            // legacy input sends the upper-case letter (`ESC X`); the program gets the legacy bytes either way.
            let c = if alt && mods.contains(KeyModifiers::SHIFT) { c.to_ascii_uppercase() } else { c };
            let mut buf = [0u8; 4];
            with_alt(c.encode_utf8(&mut buf).as_bytes().to_vec())
        }
        KeyCode::Enter => with_alt(vec![b'\r']),
        KeyCode::Tab => {
            if mods.contains(KeyModifiers::SHIFT) {
                b"\x1b[Z".to_vec()
            } else {
                with_alt(vec![b'\t'])
            }
        }
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => {
            if ctrl {
                with_alt(vec![0x08])
            } else {
                with_alt(vec![0x7f])
            }
        }
        KeyCode::Esc => with_alt(vec![0x1b]),
        KeyCode::Up => cursor(b'A'),
        KeyCode::Down => cursor(b'B'),
        KeyCode::Right => cursor(b'C'),
        KeyCode::Left => cursor(b'D'),
        KeyCode::Home => cursor(b'H'),
        KeyCode::End => cursor(b'F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n) => match n {
            1..=4 => {
                let letter = b"PQRS"[(n - 1) as usize];
                if plain { vec![0x1b, b'O', letter] } else { format!("\x1b[1;{m}{}", letter as char).into_bytes() }
            }
            5 => tilde(15),
            6 => tilde(17),
            7 => tilde(18),
            8 => tilde(19),
            9 => tilde(20),
            10 => tilde(21),
            11 => tilde(23),
            12 => tilde(24),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Encodes a mouse event per the protocol the pane requested. `col`/`row` are
/// 0-based inside the pane. `None` when the app does not want this event type.
pub fn encode_mouse(
    ev: &MouseEvent,
    col: u16,
    row: u16,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    if mode == MouseProtocolMode::None {
        return None;
    }
    let button_code = |b: MouseButton| match b {
        MouseButton::Left => 0u32,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let (mut code, release) = match ev.kind {
        MouseEventKind::Down(b) => (button_code(b), false),
        MouseEventKind::Up(b) => {
            if mode == MouseProtocolMode::Press {
                return None;
            }
            (button_code(b), true)
        }
        MouseEventKind::Drag(b) => {
            if !matches!(mode, MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion) {
                return None;
            }
            (button_code(b) + 32, false)
        }
        MouseEventKind::Moved => {
            if mode != MouseProtocolMode::AnyMotion {
                return None;
            }
            (3 + 32, false)
        }
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
    };
    if ev.modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if ev.modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if ev.modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }
    let (x, y) = (col as u32 + 1, row as u32 + 1);
    match encoding {
        MouseProtocolEncoding::Sgr => {
            let fin = if release { 'm' } else { 'M' };
            Some(format!("\x1b[<{code};{x};{y}{fin}").into_bytes())
        }
        MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
            let code = if release { 3 + (code & !3) } else { code };
            let mut out = b"\x1b[M".to_vec();
            for v in [code + 32, x + 32, y + 32] {
                if encoding == MouseProtocolEncoding::Utf8 {
                    let ch = char::from_u32(v)?;
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                } else {
                    out.push(v.min(255) as u8);
                }
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: crossterm::event::KeyEventState::NONE }
    }

    #[test]
    fn chars_and_controls() {
        assert_eq!(encode_key(&key(KeyCode::Char('a'), KeyModifiers::NONE), KeyMode::default()), b"a");
        assert_eq!(encode_key(&key(KeyCode::Char('ş'), KeyModifiers::NONE), KeyMode::default()), "ş".as_bytes());
        assert_eq!(encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), KeyMode::default()), vec![3]);
        assert_eq!(encode_key(&key(KeyCode::Char(' '), KeyModifiers::CONTROL), KeyMode::default()), vec![0]);
        assert_eq!(encode_key(&key(KeyCode::Char('b'), KeyModifiers::ALT), KeyMode::default()), b"\x1bb");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::NONE), KeyMode::default()), b"\r");
        assert_eq!(encode_key(&key(KeyCode::Backspace, KeyModifiers::NONE), KeyMode::default()), vec![0x7f]);
    }

    #[test]
    fn altgr_symbols_pass_through() {
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(encode_key(&key(KeyCode::Char('@'), altgr), KeyMode::default()), b"@");
        assert_eq!(encode_key(&key(KeyCode::Char('{'), altgr), KeyMode::default()), b"{");
        assert_eq!(encode_key(&key(KeyCode::Char('€'), altgr), KeyMode::default()), "€".as_bytes());
        // A real Ctrl+Alt+letter is still ESC + control code.
        assert_eq!(encode_key(&key(KeyCode::Char('b'), altgr), KeyMode::default()), vec![0x1b, 2]);
    }

    #[test]
    fn cursor_keys() {
        assert_eq!(encode_key(&key(KeyCode::Up, KeyModifiers::NONE), KeyMode::default()), b"\x1b[A");
        assert_eq!(
            encode_key(&key(KeyCode::Up, KeyModifiers::NONE), KeyMode { app_cursor: true, ..KeyMode::default() }),
            b"\x1bOA"
        );
        assert_eq!(encode_key(&key(KeyCode::Right, KeyModifiers::CONTROL), KeyMode::default()), b"\x1b[1;5C");
        assert_eq!(encode_key(&key(KeyCode::Delete, KeyModifiers::NONE), KeyMode::default()), b"\x1b[3~");
        assert_eq!(encode_key(&key(KeyCode::PageUp, KeyModifiers::SHIFT), KeyMode::default()), b"\x1b[5;2~");
        assert_eq!(encode_key(&key(KeyCode::F(1), KeyModifiers::NONE), KeyMode::default()), b"\x1bOP");
        assert_eq!(encode_key(&key(KeyCode::F(5), KeyModifiers::NONE), KeyMode::default()), b"\x1b[15~");
        assert_eq!(encode_key(&key(KeyCode::BackTab, KeyModifiers::SHIFT), KeyMode::default()), b"\x1b[Z");
    }

    #[test]
    fn mouse_sgr_and_default() {
        let ev = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let sgr = encode_mouse(&ev, 4, 2, MouseProtocolMode::PressRelease, MouseProtocolEncoding::Sgr).unwrap();
        assert_eq!(sgr, b"\x1b[<0;5;3M");
        let def = encode_mouse(&ev, 4, 2, MouseProtocolMode::PressRelease, MouseProtocolEncoding::Default).unwrap();
        assert_eq!(def, vec![0x1b, b'[', b'M', 32, 37, 35]);
        let up = MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), ..ev };
        assert_eq!(encode_mouse(&up, 4, 2, MouseProtocolMode::Press, MouseProtocolEncoding::Sgr), None);
        assert_eq!(
            encode_mouse(&up, 4, 2, MouseProtocolMode::PressRelease, MouseProtocolEncoding::Sgr).unwrap(),
            b"\x1b[<0;5;3m"
        );
        let wheel = MouseEvent { kind: MouseEventKind::ScrollUp, ..ev };
        assert_eq!(
            encode_mouse(&wheel, 0, 0, MouseProtocolMode::PressRelease, MouseProtocolEncoding::Sgr).unwrap(),
            b"\x1b[<64;1;1M"
        );
        assert_eq!(encode_mouse(&ev, 0, 0, MouseProtocolMode::None, MouseProtocolEncoding::Sgr), None);
    }

    fn kitty() -> KeyMode {
        KeyMode { app_cursor: false, kitty: KITTY_DISAMBIGUATE, kitty_asked: false, win32: false }
    }

    /// Without the flag, the keys that the kitty flag changes keep their legacy bytes. A flag NOBLE does not support
    /// changes nothing either.
    #[test]
    fn kitty_keys_are_legacy_without_the_flag() {
        let other_flags = KeyMode { app_cursor: false, kitty: 2, kitty_asked: false, win32: false };
        for mode in [KeyMode::default(), other_flags] {
            assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::NONE), mode), b"\x1b");
            assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), mode), b"\r");
            assert_eq!(encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), mode), vec![3]);
            assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::CONTROL), mode), b"\t");
            assert_eq!(encode_key(&key(KeyCode::Backspace, KeyModifiers::CONTROL), mode), vec![0x08]);
        }
    }

    /// Esc, and Enter, Tab and Backspace with a modifier, become CSI u; without one they keep their bytes.
    #[test]
    fn kitty_escape_enter_tab_and_backspace() {
        let k = kitty();
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::NONE), k), b"\x1b[27u");
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::ALT), k), b"\x1b[27;3u");
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::CONTROL), k), b"\x1b[27;5u");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::NONE), k), b"\r");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), k), b"\x1b[13;2u");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL), k), b"\x1b[13;5u");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::ALT), k), b"\x1b[13;3u");
        assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::NONE), k), b"\t");
        assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::CONTROL), k), b"\x1b[9;5u");
        assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::SHIFT), k), b"\x1b[9;2u");
        assert_eq!(encode_key(&key(KeyCode::BackTab, KeyModifiers::SHIFT), k), b"\x1b[9;2u");
        assert_eq!(encode_key(&key(KeyCode::BackTab, KeyModifiers::NONE), k), b"\x1b[9;2u");
        assert_eq!(encode_key(&key(KeyCode::Backspace, KeyModifiers::NONE), k), vec![0x7f]);
        assert_eq!(encode_key(&key(KeyCode::Backspace, KeyModifiers::CONTROL), k), b"\x1b[127;5u");
        assert_eq!(encode_key(&key(KeyCode::Backspace, KeyModifiers::ALT), k), b"\x1b[127;3u");
    }

    /// Text keys send their text with no modifier or Shift only; with Ctrl or Alt they send CSI u, with the
    /// lower-case letter (or the character itself) as the code.
    #[test]
    fn kitty_text_keys() {
        let k = kitty();
        assert_eq!(encode_key(&key(KeyCode::Char('a'), KeyModifiers::NONE), k), b"a");
        assert_eq!(encode_key(&key(KeyCode::Char('A'), KeyModifiers::SHIFT), k), b"A");
        assert_eq!(encode_key(&key(KeyCode::Char('ş'), KeyModifiers::NONE), k), "ş".as_bytes());
        assert_eq!(encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), k), b"\x1b[99;5u");
        assert_eq!(encode_key(&key(KeyCode::Char('A'), KeyModifiers::CONTROL | KeyModifiers::SHIFT), k), b"\x1b[97;6u");
        assert_eq!(encode_key(&key(KeyCode::Char('['), KeyModifiers::CONTROL), k), b"\x1b[91;5u");
        assert_eq!(encode_key(&key(KeyCode::Char('1'), KeyModifiers::CONTROL), k), b"\x1b[49;5u");
        assert_eq!(encode_key(&key(KeyCode::Char('ş'), KeyModifiers::CONTROL), k), b"\x1b[351;5u");
        assert_eq!(encode_key(&key(KeyCode::Char('b'), KeyModifiers::ALT), k), b"\x1b[98;3u");
        assert_eq!(encode_key(&key(KeyCode::Char('b'), KeyModifiers::CONTROL | KeyModifiers::ALT), k), b"\x1b[98;7u");
        assert_eq!(encode_key(&key(KeyCode::Char(' '), KeyModifiers::CONTROL), k), b"\x1b[32;5u");
        assert_eq!(encode_key(&key(KeyCode::Char('/'), KeyModifiers::CONTROL), k), b"\x1b[47;5u");
        assert_eq!(encode_key(&key(KeyCode::Char('a'), KeyModifiers::SUPER), k), b"\x1b[97;9u");
    }

    /// AltGr characters stay text under the flag too (the Windows layouts send them as Ctrl+Alt).
    #[test]
    fn kitty_altgr_characters_stay_text() {
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(encode_key(&key(KeyCode::Char('@'), altgr), kitty()), b"@");
        assert_eq!(encode_key(&key(KeyCode::Char('€'), altgr), kitty()), "€".as_bytes());
    }

    /// Ctrl+Shift+letter is one key in both shapes: legacy input sends `Char('C')`, the kitty protocol `Char('c')`
    /// (`CSI 99 ; 6 u`). A program that asked for nothing gets the same control code for both.
    #[test]
    fn ctrl_shift_letter_is_the_same_in_both_shapes() {
        let both = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        for mode in [KeyMode::default(), kitty()] {
            assert_eq!(
                encode_key(&key(KeyCode::Char('c'), both), mode),
                encode_key(&key(KeyCode::Char('C'), both), mode),
                "{mode:?}"
            );
        }
        assert_eq!(encode_key(&key(KeyCode::Char('C'), both), KeyMode::default()), vec![3]);
        assert_eq!(encode_key(&key(KeyCode::Char('c'), both), kitty()), b"\x1b[99;6u");
    }

    /// Alt+Shift+letter: the kitty shape (lower case with SHIFT, `CSI 120 ; 4 u`) reaches a program that pushed nothing
    /// as `ESC X`, the bytes legacy input sends; a program that pushed the flag gets the CSI u form in both shapes.
    #[test]
    fn alt_shift_letter_is_the_same_in_both_shapes() {
        let alt_shift = KeyModifiers::ALT | KeyModifiers::SHIFT;
        for ch in ['x', 'X'] {
            assert_eq!(encode_key(&key(KeyCode::Char(ch), alt_shift), KeyMode::default()), b"\x1bX");
            assert_eq!(encode_key(&key(KeyCode::Char(ch), alt_shift), kitty()), b"\x1b[120;4u");
        }
        assert_eq!(encode_key(&key(KeyCode::Char('X'), KeyModifiers::ALT), KeyMode::default()), b"\x1bX");
        // Shift alone keeps the typed letter as it is.
        assert_eq!(encode_key(&key(KeyCode::Char('x'), KeyModifiers::SHIFT), KeyMode::default()), b"x");
    }

    /// Alt+Shift+letter from the alternate-keys shape (`Char('X')` with ALT, Shift moved into the letter) keeps its
    /// Shift for a kitty program: `CSI 120 ; 4 u`, the same as the plain shape gives.
    #[test]
    fn kitty_alt_shift_letter_keeps_shift_in_the_alternate_keys_shape() {
        assert_eq!(encode_key(&key(KeyCode::Char('X'), KeyModifiers::ALT), kitty()), b"\x1b[120;4u");
        assert_eq!(encode_key(&key(KeyCode::Char('C'), KeyModifiers::CONTROL), kitty()), b"\x1b[99;6u");
        assert_eq!(encode_key(&key(KeyCode::Char('C'), KeyModifiers::CONTROL), KeyMode::default()), vec![3]);
    }

    /// Ctrl+/ is the same key in both shapes: legacy input sends it as 0x1f (crossterm reads it as `Char('7')` with
    /// CONTROL), and the kitty form is `Char('/')` with CONTROL. A program that asked for nothing gets 0x1f for both.
    /// Ctrl+Shift+/ comes as `?` with the alternate-keys flag, and sends 0x7f as `?` always has.
    #[test]
    fn ctrl_slash_is_the_same_in_both_shapes() {
        let legacy = KeyMode::default();
        assert_eq!(encode_key(&key(KeyCode::Char('/'), KeyModifiers::CONTROL), legacy), vec![0x1f]);
        assert_eq!(encode_key(&key(KeyCode::Char('7'), KeyModifiers::CONTROL), legacy), vec![0x1f]);
        assert_eq!(encode_key(&key(KeyCode::Char('?'), KeyModifiers::CONTROL), legacy), vec![0x7f]);
        // A program that pushed the flag gets the CSI u form of the kitty shape.
        assert_eq!(encode_key(&key(KeyCode::Char('/'), KeyModifiers::CONTROL), kitty()), b"\x1b[47;5u");
    }

    /// A symbol typed with Alt keeps its character for a program that asked for nothing (`ESC !`), and the kitty
    /// form carries the character as the code (`CSI 33 ; 3 u`).
    #[test]
    fn alt_symbol_is_the_character_in_both_shapes() {
        assert_eq!(encode_key(&key(KeyCode::Char('!'), KeyModifiers::ALT), KeyMode::default()), b"\x1b!");
        assert_eq!(encode_key(&key(KeyCode::Char('!'), KeyModifiers::ALT), kitty()), b"\x1b[33;3u");
    }

    /// Super, Hyper and Meta keys: legacy input has no bytes for them, so a program that asked for nothing gets
    /// nothing (not the bare letter). The kitty form carries the modifier bit (Super 8, Hyper 16, Meta 32).
    #[test]
    fn super_hyper_and_meta_keys_are_only_sent_in_the_kitty_form() {
        for mods in [KeyModifiers::SUPER, KeyModifiers::HYPER, KeyModifiers::META] {
            assert!(encode_key(&key(KeyCode::Char('k'), mods), KeyMode::default()).is_empty(), "{mods:?}");
            assert!(encode_key(&key(KeyCode::Enter, mods), KeyMode::default()).is_empty(), "{mods:?}");
        }
        assert_eq!(encode_key(&key(KeyCode::Char('k'), KeyModifiers::SUPER), kitty()), b"\x1b[107;9u");
        assert_eq!(encode_key(&key(KeyCode::Char('k'), KeyModifiers::HYPER), kitty()), b"\x1b[107;17u");
        assert_eq!(encode_key(&key(KeyCode::Char('k'), KeyModifiers::META), kitty()), b"\x1b[107;33u");
    }

    /// The keys with a sequence of their own are the same with the flag: cursor keys (with DECCKM), editing and
    /// function keys.
    #[test]
    fn kitty_leaves_cursor_editing_and_function_keys_alone() {
        let k = kitty();
        let app = KeyMode { app_cursor: true, kitty: KITTY_DISAMBIGUATE, kitty_asked: false, win32: false };
        assert_eq!(encode_key(&key(KeyCode::Up, KeyModifiers::NONE), k), b"\x1b[A");
        assert_eq!(encode_key(&key(KeyCode::Up, KeyModifiers::NONE), app), b"\x1bOA");
        assert_eq!(encode_key(&key(KeyCode::Right, KeyModifiers::CONTROL), k), b"\x1b[1;5C");
        assert_eq!(encode_key(&key(KeyCode::Home, KeyModifiers::NONE), k), b"\x1b[H");
        assert_eq!(encode_key(&key(KeyCode::End, KeyModifiers::NONE), k), b"\x1b[F");
        assert_eq!(encode_key(&key(KeyCode::Delete, KeyModifiers::NONE), k), b"\x1b[3~");
        assert_eq!(encode_key(&key(KeyCode::PageUp, KeyModifiers::SHIFT), k), b"\x1b[5;2~");
        assert_eq!(encode_key(&key(KeyCode::F(1), KeyModifiers::NONE), k), b"\x1bOP");
        assert_eq!(encode_key(&key(KeyCode::F(5), KeyModifiers::NONE), k), b"\x1b[15~");
    }

    fn win32() -> KeyMode {
        KeyMode { win32: true, ..KeyMode::default() }
    }

    /// The key-down and key-up records of one key, as ConPTY reads them.
    fn records(vk: u8, sc: u8, uc: u8, cs: u8) -> Vec<u8> {
        format!("\x1b[{vk};{sc};{uc};1;{cs};1_\x1b[{vk};{sc};{uc};0;{cs};1_").into_bytes()
    }

    /// Every key of the win32 set, with each modifier combination the set allows. The byte strings are the ones a
    /// Windows console reports (Cs: Shift 0x10, Ctrl 0x08, Alt 0x02).
    #[test]
    fn win32_keys_are_console_records() {
        let w = win32();
        // Enter: Shift and/or Ctrl, Alt may be added. Ctrl makes the character LF (10).
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), w), records(13, 28, 13, 16));
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL), w), records(13, 28, 10, 8));
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT), w),
            records(13, 28, 10, 24)
        );
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::ALT), w),
            records(13, 28, 10, 10)
        );
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT | KeyModifiers::ALT), w),
            records(13, 28, 13, 18)
        );
        // Esc: any modifiers.
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::NONE), w), records(27, 1, 27, 0));
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::SHIFT), w), records(27, 1, 27, 16));
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::CONTROL), w), records(27, 1, 27, 8));
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::ALT), w), records(27, 1, 27, 2));
        // Tab: Ctrl, Shift may be added. Backspace: Ctrl, Shift may be added; Ctrl makes the character DEL (127).
        assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::CONTROL), w), records(9, 15, 9, 8));
        assert_eq!(
            encode_key(&key(KeyCode::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT), w),
            records(9, 15, 9, 24)
        );
        assert_eq!(encode_key(&key(KeyCode::Backspace, KeyModifiers::CONTROL), w), records(8, 14, 127, 8));
        assert_eq!(
            encode_key(&key(KeyCode::Backspace, KeyModifiers::CONTROL | KeyModifiers::SHIFT), w),
            records(8, 14, 127, 24)
        );
    }

    /// crossterm reports Ctrl+Shift+Tab as the back tab with Ctrl and Shift; Shift is part of Cs either way.
    #[test]
    fn win32_back_tab_with_ctrl_is_a_tab_record() {
        let w = win32();
        assert_eq!(encode_key(&key(KeyCode::BackTab, KeyModifiers::CONTROL), w), records(9, 15, 9, 24));
        assert_eq!(
            encode_key(&key(KeyCode::BackTab, KeyModifiers::CONTROL | KeyModifiers::SHIFT), w),
            records(9, 15, 9, 24)
        );
        // Without Ctrl the back tab stays CSI Z; with Ctrl and Alt it keeps the legacy bytes too (the back tab never
        // had an Alt prefix).
        assert_eq!(encode_key(&key(KeyCode::BackTab, KeyModifiers::SHIFT), w), b"\x1b[Z");
        assert_eq!(encode_key(&key(KeyCode::BackTab, KeyModifiers::CONTROL | KeyModifiers::ALT), w), b"\x1b[Z");
    }

    /// The record is the key down followed by the key up, both with the same fields: the byte strings ConPTY
    /// documents for win32-input-mode.
    #[test]
    fn win32_record_bytes_match_the_documented_sequences() {
        let w = win32();
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), w),
            b"\x1b[13;28;13;1;16;1_\x1b[13;28;13;0;16;1_"
        );
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL), w),
            b"\x1b[13;28;10;1;8;1_\x1b[13;28;10;0;8;1_"
        );
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::NONE), w), b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_");
        assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::CONTROL), w), b"\x1b[9;15;9;1;8;1_\x1b[9;15;9;0;8;1_");
        assert_eq!(
            encode_key(&key(KeyCode::Backspace, KeyModifiers::CONTROL), w),
            b"\x1b[8;14;127;1;8;1_\x1b[8;14;127;0;8;1_"
        );
    }

    /// Keys outside the set keep their legacy bytes with win32-input-mode on: plain Enter, Enter with Alt only, plain
    /// and Shift Tab, Tab with Ctrl and Alt, Backspace without Ctrl, Ctrl+c and the cursor keys.
    #[test]
    fn win32_leaves_other_keys_legacy() {
        let w = win32();
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::NONE), w), b"\r");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::ALT), w), b"\x1b\r");
        assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::NONE), w), b"\t");
        assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::SHIFT), w), b"\x1b[Z");
        assert_eq!(encode_key(&key(KeyCode::BackTab, KeyModifiers::SHIFT), w), b"\x1b[Z");
        assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::CONTROL | KeyModifiers::ALT), w), b"\x1b\t");
        assert_eq!(encode_key(&key(KeyCode::Backspace, KeyModifiers::NONE), w), vec![0x7f]);
        assert_eq!(encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), w), vec![3]);
        assert_eq!(encode_key(&key(KeyCode::Char('a'), KeyModifiers::NONE), w), b"a");
        assert_eq!(encode_key(&key(KeyCode::Up, KeyModifiers::NONE), w), b"\x1b[A");
        assert_eq!(encode_key(&key(KeyCode::Right, KeyModifiers::CONTROL), w), b"\x1b[1;5C");
    }

    /// Precedence: with the kitty flag on, the kitty encoding wins over win32-input-mode for the same keys; without
    /// the mode, the kitty encoding is unchanged.
    #[test]
    fn kitty_beats_win32_for_the_same_keys() {
        let both = KeyMode { app_cursor: false, kitty: KITTY_DISAMBIGUATE, kitty_asked: false, win32: true };
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::NONE), both), b"\x1b[27u");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), both), b"\x1b[13;2u");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL), both), b"\x1b[13;5u");
        assert_eq!(encode_key(&key(KeyCode::Tab, KeyModifiers::CONTROL), both), b"\x1b[9;5u");
        assert_eq!(encode_key(&key(KeyCode::Backspace, KeyModifiers::CONTROL), both), b"\x1b[127;5u");
    }

    /// A program that asked for the kitty flags but has no flag 1 (its answer may have been lost) gets Shift+Enter as
    /// `CSI 13 ; 2 u`, where the legacy `\r` would submit its prompt. Only Shift alone is changed: every other Enter
    /// keeps its legacy bytes, and Super, Hyper and Meta still send nothing.
    #[test]
    fn asked_shift_enter_falls_back_to_csi_u() {
        let asked = KeyMode { kitty_asked: true, ..KeyMode::default() };
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), asked), b"\x1b[13;2u");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::NONE), asked), b"\r");
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL), asked),
            b"\r",
            "Ctrl+Enter is not the fallback"
        );
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT), asked), b"\r");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::ALT | KeyModifiers::SHIFT), asked), b"\x1b\r");
        assert!(encode_key(&key(KeyCode::Enter, KeyModifiers::SUPER | KeyModifiers::SHIFT), asked).is_empty());
        assert!(encode_key(&key(KeyCode::Enter, KeyModifiers::HYPER | KeyModifiers::SHIFT), asked).is_empty());
        assert!(encode_key(&key(KeyCode::Enter, KeyModifiers::META | KeyModifiers::SHIFT), asked).is_empty());
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::NONE), asked), b"\x1b", "other keys keep their bytes");
    }

    /// With flag 1 pushed the kitty form comes from the flag itself, and the fallback adds nothing there.
    #[test]
    fn flag_one_still_wins_over_the_fallback() {
        let both = KeyMode { kitty: KITTY_DISAMBIGUATE, kitty_asked: true, ..KeyMode::default() };
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), both), b"\x1b[13;2u");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL), both), b"\x1b[13;5u");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::NONE), both), b"\r");
    }

    /// The fallback comes before win32-input-mode: Shift+Enter is the CSI u form, not the console record, while
    /// Ctrl+Enter (outside the fallback) still gets its record.
    #[test]
    fn asked_fallback_beats_the_win32_record_for_shift_enter() {
        let asked_win32 = KeyMode { kitty_asked: true, win32: true, ..KeyMode::default() };
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), asked_win32), b"\x1b[13;2u");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::NONE), asked_win32), b"\r");
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::CONTROL), asked_win32),
            b"\x1b[13;28;10;1;8;1_\x1b[13;28;10;0;8;1_"
        );
    }

    /// Without the ask, Shift+Enter keeps today's bytes: the legacy `\r`, or the console record under win32-input-mode.
    #[test]
    fn shift_enter_without_the_ask_is_unchanged() {
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), KeyMode::default()), b"\r");
        let flag_two = KeyMode { kitty: 2, ..KeyMode::default() };
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), flag_two), b"\r");
        let win32_only = KeyMode { win32: true, ..KeyMode::default() };
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), win32_only),
            b"\x1b[13;28;13;1;16;1_\x1b[13;28;13;0;16;1_"
        );
    }
}
