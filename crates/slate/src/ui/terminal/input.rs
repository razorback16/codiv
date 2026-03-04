use crossterm::event::{KeyCode, KeyModifiers};

/// Convert a crossterm key event into raw terminal bytes for PTY forwarding.
///
/// Handles the full range of keys: printable characters (with Ctrl/Alt
/// modifiers), navigation keys, function keys, and editing keys. Returns
/// `None` only for keys that have no byte representation (e.g. bare
/// modifier presses).
pub(crate) fn key_event_to_bytes(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    match code {
        KeyCode::Char(ch) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                // Ctrl+letter → 0x01..0x1A
                let ctrl = (ch.to_ascii_lowercase() as u8).wrapping_sub(b'a').wrapping_add(1);
                if ctrl <= 26 {
                    Some(vec![ctrl])
                } else {
                    None
                }
            } else if modifiers.contains(KeyModifiers::ALT) {
                let mut buf = vec![0x1b]; // ESC prefix
                let mut tmp = [0u8; 4];
                buf.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                Some(buf)
            } else {
                let mut tmp = [0u8; 4];
                let s = ch.encode_utf8(&mut tmp);
                Some(s.as_bytes().to_vec())
            }
        }
        KeyCode::Enter => Some(b"\r".to_vec()),
        KeyCode::Backspace => Some(b"\x7f".to_vec()),
        KeyCode::Tab => Some(b"\t".to_vec()),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Esc => Some(b"\x1b".to_vec()),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        KeyCode::Insert => Some(b"\x1b[2~".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::F(n) => {
            let s = match n {
                1 => "\x1bOP",
                2 => "\x1bOQ",
                3 => "\x1bOR",
                4 => "\x1bOS",
                5 => "\x1b[15~",
                6 => "\x1b[17~",
                7 => "\x1b[18~",
                8 => "\x1b[19~",
                9 => "\x1b[20~",
                10 => "\x1b[21~",
                11 => "\x1b[23~",
                12 => "\x1b[24~",
                _ => return None,
            };
            Some(s.as_bytes().to_vec())
        }
        _ => None,
    }
}

/// Find the longest common prefix of a list of strings.
pub(crate) fn longest_common_prefix(strings: &[String]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    let mut prefix = strings[0].clone();
    for s in &strings[1..] {
        let shared: String = prefix
            .chars()
            .zip(s.chars())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a)
            .collect();
        prefix = shared;
    }
    prefix
}
