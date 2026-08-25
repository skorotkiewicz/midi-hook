use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{self, Write};

struct RawMode;

impl RawMode {
    fn enable() -> Result<Self, String> {
        enable_raw_mode().map_err(|error| format!("could not read keyboard shortcut: {error}"))?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

pub(crate) fn capture_ydotool_command() -> Result<String, String> {
    print!("Press the computer key or shortcut to map: ");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let raw_mode = RawMode::enable()?;
    let key = loop {
        match event::read().map_err(|error| error.to_string())? {
            Event::Key(key) if key.kind == KeyEventKind::Press => break key,
            _ => {}
        }
    };
    drop(raw_mode);
    println!();
    ydotool_command(key)
}

fn ydotool_command(key: KeyEvent) -> Result<String, String> {
    if key
        .modifiers
        .intersects(KeyModifiers::HYPER | KeyModifiers::META)
    {
        return Err("Hyper and Meta shortcuts are not supported".into());
    }
    let key_code =
        linux_key_code(key.code).ok_or_else(|| format!("unsupported key: {:?}", key.code))?;
    let mut modifiers = Vec::new();
    for (modifier, code) in [
        (KeyModifiers::CONTROL, 29),
        (KeyModifiers::ALT, 56),
        (KeyModifiers::SHIFT, 42),
        (KeyModifiers::SUPER, 125),
    ] {
        if key.modifiers.contains(modifier) {
            modifiers.push(code);
        }
    }

    let mut events: Vec<String> = modifiers.iter().map(|code| format!("{code}:1")).collect();
    events.push(format!("{key_code}:1"));
    events.push(format!("{key_code}:0"));
    events.extend(modifiers.iter().rev().map(|code| format!("{code}:0")));
    Ok(format!("ydotool key {}", events.join(" ")))
}

fn linux_key_code(key: KeyCode) -> Option<u16> {
    Some(match key {
        KeyCode::Esc => 1,
        KeyCode::Char('1') => 2,
        KeyCode::Char('2') => 3,
        KeyCode::Char('3') => 4,
        KeyCode::Char('4') => 5,
        KeyCode::Char('5') => 6,
        KeyCode::Char('6') => 7,
        KeyCode::Char('7') => 8,
        KeyCode::Char('8') => 9,
        KeyCode::Char('9') => 10,
        KeyCode::Char('0') => 11,
        KeyCode::Backspace => 14,
        KeyCode::Tab | KeyCode::BackTab => 15,
        KeyCode::Char('q' | 'Q') => 16,
        KeyCode::Char('w' | 'W') => 17,
        KeyCode::Char('e' | 'E') => 18,
        KeyCode::Char('r' | 'R') => 19,
        KeyCode::Char('t' | 'T') => 20,
        KeyCode::Char('y' | 'Y') => 21,
        KeyCode::Char('u' | 'U') => 22,
        KeyCode::Char('i' | 'I') => 23,
        KeyCode::Char('o' | 'O') => 24,
        KeyCode::Char('p' | 'P') => 25,
        KeyCode::Enter => 28,
        KeyCode::Char('a' | 'A') => 30,
        KeyCode::Char('s' | 'S') => 31,
        KeyCode::Char('d' | 'D') => 32,
        KeyCode::Char('f' | 'F') => 33,
        KeyCode::Char('g' | 'G') => 34,
        KeyCode::Char('h' | 'H') => 35,
        KeyCode::Char('j' | 'J') => 36,
        KeyCode::Char('k' | 'K') => 37,
        KeyCode::Char('l' | 'L') => 38,
        KeyCode::Char('z' | 'Z') => 44,
        KeyCode::Char('x' | 'X') => 45,
        KeyCode::Char('c' | 'C') => 46,
        KeyCode::Char('v' | 'V') => 47,
        KeyCode::Char('b' | 'B') => 48,
        KeyCode::Char('n' | 'N') => 49,
        KeyCode::Char('m' | 'M') => 50,
        KeyCode::Char(' ') => 57,
        KeyCode::F(number @ 1..=10) => 58 + u16::from(number),
        KeyCode::F(11) => 87,
        KeyCode::F(12) => 88,
        KeyCode::Home => 102,
        KeyCode::Up => 103,
        KeyCode::PageUp => 104,
        KeyCode::Left => 105,
        KeyCode::Right => 106,
        KeyCode::End => 107,
        KeyCode::Down => 108,
        KeyCode::PageDown => 109,
        KeyCode::Insert => 110,
        KeyCode::Delete => 111,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_shortcuts_to_ydotool_events() {
        assert_eq!(
            ydotool_command(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap(),
            "ydotool key 29:1 46:1 46:0 29:0"
        );
        assert_eq!(
            ydotool_command(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)).unwrap(),
            "ydotool key 57:1 57:0"
        );
        assert_eq!(
            ydotool_command(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).unwrap(),
            "ydotool key 28:1 28:0"
        );
    }
}
