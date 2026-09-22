//! Klavye ve fare olaylarını PTY'ye gönderilecek xterm bayt dizilerine çevirir.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// xterm değiştirici parametresi: 1 + shift + 2·alt + 4·ctrl.
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

/// Ctrl+Alt ile gelen, harf olmayan karakter AltGr ile üretilmiştir.
pub fn is_altgr_char(mods: KeyModifiers, c: char) -> bool {
    mods.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) && !c.is_ascii_alphabetic()
}

/// Tuş olayını baytlara çevirir. `app_cursor`: DECCKM (uygulama imleç modu).
pub fn encode_key(ev: &KeyEvent, app_cursor: bool) -> Vec<u8> {
    let mut mods = ev.modifiers;
    // Windows AltGr = Ctrl+Alt: '@', '{', '|', '€' gibi üretilmiş karakterler
    // olduğu gibi gönderilir, kontrol koduna çevrilmez.
    if let KeyCode::Char(c) = ev.code
        && is_altgr_char(mods, c)
    {
        mods = KeyModifiers::NONE;
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
    // CSI imleç tuşları: ESC [ x / ESC O x / ESC [ 1 ; m x
    let cursor = |letter: u8| -> Vec<u8> {
        if plain {
            if app_cursor { vec![0x1b, b'O', letter] } else { vec![0x1b, b'[', letter] }
        } else {
            format!("\x1b[1;{m}{}", letter as char).into_bytes()
        }
    };
    // Tilde tuşları: ESC [ n ~ / ESC [ n ; m ~
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
                    '_' | '-' | '7' => Some(0x1f),
                    '?' | '8' => Some(0x7f),
                    _ => None,
                };
                if let Some(b) = byte {
                    return with_alt(vec![b]);
                }
            }
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

/// Fare olayını, pane'in istediği protokole göre kodlar. `col`/`row` pane
/// içine göre 0 tabanlıdır. Uygulama bu olay türünü istemiyorsa `None`.
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
        assert_eq!(encode_key(&key(KeyCode::Char('a'), KeyModifiers::NONE), false), b"a");
        assert_eq!(encode_key(&key(KeyCode::Char('ş'), KeyModifiers::NONE), false), "ş".as_bytes());
        assert_eq!(encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), false), vec![3]);
        assert_eq!(encode_key(&key(KeyCode::Char(' '), KeyModifiers::CONTROL), false), vec![0]);
        assert_eq!(encode_key(&key(KeyCode::Char('b'), KeyModifiers::ALT), false), b"\x1bb");
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::NONE), false), b"\r");
        assert_eq!(encode_key(&key(KeyCode::Backspace, KeyModifiers::NONE), false), vec![0x7f]);
    }

    #[test]
    fn altgr_symbols_pass_through() {
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(encode_key(&key(KeyCode::Char('@'), altgr), false), b"@");
        assert_eq!(encode_key(&key(KeyCode::Char('{'), altgr), false), b"{");
        assert_eq!(encode_key(&key(KeyCode::Char('€'), altgr), false), "€".as_bytes());
        // Gerçek Ctrl+Alt+harf hâlâ ESC + kontrol kodu.
        assert_eq!(encode_key(&key(KeyCode::Char('b'), altgr), false), vec![0x1b, 2]);
    }

    #[test]
    fn cursor_keys() {
        assert_eq!(encode_key(&key(KeyCode::Up, KeyModifiers::NONE), false), b"\x1b[A");
        assert_eq!(encode_key(&key(KeyCode::Up, KeyModifiers::NONE), true), b"\x1bOA");
        assert_eq!(encode_key(&key(KeyCode::Right, KeyModifiers::CONTROL), false), b"\x1b[1;5C");
        assert_eq!(encode_key(&key(KeyCode::Delete, KeyModifiers::NONE), false), b"\x1b[3~");
        assert_eq!(encode_key(&key(KeyCode::PageUp, KeyModifiers::SHIFT), false), b"\x1b[5;2~");
        assert_eq!(encode_key(&key(KeyCode::F(1), KeyModifiers::NONE), false), b"\x1bOP");
        assert_eq!(encode_key(&key(KeyCode::F(5), KeyModifiers::NONE), false), b"\x1b[15~");
        assert_eq!(encode_key(&key(KeyCode::BackTab, KeyModifiers::SHIFT), false), b"\x1b[Z");
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
}
