use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Encode a key event as the bytes an xterm-compatible program expects on
/// stdin. Used to forward keystrokes from our UI into the agent's PTY.
pub fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
    let mods = key.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let shift = mods.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Char(c) => {
            if ctrl && !alt {
                let lower = c.to_ascii_lowercase();
                let control = match lower {
                    ' ' | '@' => Some(0u8),
                    'a'..='z' => Some(lower as u8 - b'a' + 1),
                    '[' => Some(0x1b),
                    '\\' => Some(0x1c),
                    ']' => Some(0x1d),
                    '^' => Some(0x1e),
                    '_' | '?' => Some(0x1f),
                    _ => None,
                };
                if let Some(byte) = control {
                    return vec![byte];
                }
            }
            let mut text = String::new();
            if alt {
                text.push('\x1b');
            }
            text.push(c);
            text.into_bytes()
        }
        KeyCode::Enter => {
            if alt {
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        KeyCode::Tab => {
            if shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => {
            if alt {
                b"\x1b\x7f".to_vec()
            } else {
                b"\x7f".to_vec()
            }
        }
        KeyCode::Esc => b"\x1b".to_vec(),
        KeyCode::Up => arrow('A', shift, alt, ctrl),
        KeyCode::Down => arrow('B', shift, alt, ctrl),
        KeyCode::Right => arrow('C', shift, alt, ctrl),
        KeyCode::Left => arrow('D', shift, alt, ctrl),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::F(number) => function_key(number),
        _ => Vec::new(),
    }
}

fn arrow(letter: char, shift: bool, alt: bool, ctrl: bool) -> Vec<u8> {
    let mut modifier = 1;
    if shift {
        modifier += 1;
    }
    if alt {
        modifier += 2;
    }
    if ctrl {
        modifier += 4;
    }
    if modifier == 1 {
        format!("\x1b[{letter}").into_bytes()
    } else {
        format!("\x1b[1;{modifier}{letter}").into_bytes()
    }
}

fn function_key(number: u8) -> Vec<u8> {
    match number {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        n => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return Vec::new(),
            };
            format!("\x1b[{code}~").into_bytes()
        }
    }
}
