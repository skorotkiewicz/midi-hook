use evdev::KeyCode;
use std::collections::HashSet;

fn named_key_code(name: &str) -> Option<u16> {
    let key = match name.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => KeyCode::KEY_LEFTCTRL,
        "alt" => KeyCode::KEY_LEFTALT,
        "shift" => KeyCode::KEY_LEFTSHIFT,
        "super" | "win" | "windows" => KeyCode::KEY_LEFTMETA,
        "space" => KeyCode::KEY_SPACE,
        "enter" | "return" => KeyCode::KEY_ENTER,
        "esc" | "escape" => KeyCode::KEY_ESC,
        "tab" => KeyCode::KEY_TAB,
        "backspace" => KeyCode::KEY_BACKSPACE,
        "a" => KeyCode::KEY_A,
        "b" => KeyCode::KEY_B,
        "c" => KeyCode::KEY_C,
        "d" => KeyCode::KEY_D,
        "e" => KeyCode::KEY_E,
        "f" => KeyCode::KEY_F,
        "g" => KeyCode::KEY_G,
        "h" => KeyCode::KEY_H,
        "i" => KeyCode::KEY_I,
        "j" => KeyCode::KEY_J,
        "k" => KeyCode::KEY_K,
        "l" => KeyCode::KEY_L,
        "m" => KeyCode::KEY_M,
        "n" => KeyCode::KEY_N,
        "o" => KeyCode::KEY_O,
        "p" => KeyCode::KEY_P,
        "q" => KeyCode::KEY_Q,
        "r" => KeyCode::KEY_R,
        "s" => KeyCode::KEY_S,
        "t" => KeyCode::KEY_T,
        "u" => KeyCode::KEY_U,
        "v" => KeyCode::KEY_V,
        "w" => KeyCode::KEY_W,
        "x" => KeyCode::KEY_X,
        "y" => KeyCode::KEY_Y,
        "z" => KeyCode::KEY_Z,
        "f1" => KeyCode::KEY_F1,
        "f2" => KeyCode::KEY_F2,
        "f3" => KeyCode::KEY_F3,
        "f4" => KeyCode::KEY_F4,
        "f5" => KeyCode::KEY_F5,
        "f6" => KeyCode::KEY_F6,
        "f7" => KeyCode::KEY_F7,
        "f8" => KeyCode::KEY_F8,
        "f9" => KeyCode::KEY_F9,
        "f10" => KeyCode::KEY_F10,
        "f11" => KeyCode::KEY_F11,
        "f12" => KeyCode::KEY_F12,
        _ => return name.trim().parse().ok(),
    };
    Some(key.code())
}

pub(crate) fn parse_shortcut(value: &str, line: usize) -> Result<Vec<(u16, i32)>, String> {
    if value.contains('+') {
        let codes: Result<Vec<_>, _> = value
            .split('+')
            .map(|name| {
                named_key_code(name)
                    .ok_or_else(|| format!("line {line}: unsupported key name: {name}"))
            })
            .collect();
        let codes = codes?;
        if codes.len() < 2 {
            return Err(format!("line {line}: shortcut needs at least two keys"));
        }
        let mut events: Vec<_> = codes.iter().map(|code| (*code, 1)).collect();
        events.extend(codes.iter().rev().map(|code| (*code, 0)));
        return Ok(events);
    }

    let mut active = HashSet::new();
    let mut events = Vec::new();
    for token in value.split_whitespace() {
        let (code, value) = token
            .split_once(':')
            .ok_or_else(|| format!("line {line}: expected KEY_CODE:VALUE"))?;
        let code = code
            .parse::<u16>()
            .map_err(|_| format!("line {line}: invalid key code: {code}"))?;
        let value = value
            .parse::<i32>()
            .ok()
            .filter(|value| matches!(value, 0 | 1))
            .ok_or_else(|| format!("line {line}: key value must be 0 or 1"))?;
        if (value == 1 && !active.insert(code)) || (value == 0 && !active.remove(&code)) {
            return Err(format!("line {line}: unbalanced key event: {token}"));
        }
        events.push((code, value));
    }
    if events.is_empty() || !active.is_empty() {
        return Err(format!("line {line}: shortcut must release every key"));
    }
    Ok(events)
}
