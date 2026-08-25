use crate::keyboard::named_key_code;
use std::collections::HashSet;

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
