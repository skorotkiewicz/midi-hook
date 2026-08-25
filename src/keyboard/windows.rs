use super::{Capture, RawMode, terminal_escape_pressed};
use std::mem::size_of;
use std::thread;
use std::time::Duration;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput,
};

pub(crate) struct Keyboard;

pub(crate) fn choose_keyboard() -> Result<Keyboard, String> {
    println!("Using the system keyboard.");
    Ok(Keyboard)
}

fn is_pressed(code: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(code)) < 0 }
}

pub(crate) fn capture_keyboard_action(_: &Keyboard) -> Result<Option<String>, String> {
    println!("Hold the shortcut keys, then release them all. Press Esc to cancel.");
    let _raw_mode = RawMode::enable()?;
    let mut previous = [false; 256];
    let mut capture = Capture::default();
    loop {
        if is_pressed(0x1b) || terminal_escape_pressed()? {
            return Ok(None);
        }
        for code in 7u16..=254 {
            if matches!(code, 0x10..=0x12) {
                continue;
            }
            let pressed = is_pressed(code);
            if pressed != previous[usize::from(code)] {
                previous[usize::from(code)] = pressed;
                if capture.push(code, i32::from(pressed)) {
                    return Ok(Some(capture.action()));
                }
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub(crate) fn run_held_key(code: u16, pressed: bool) -> Result<(), String> {
    let mut flags = if pressed { 0 } else { KEYEVENTF_KEYUP };
    if matches!(
        code,
        0x21..=0x2e | 0x5b..=0x5d | 0x6f | 0x90 | 0xa3 | 0xa5
    ) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: code,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    if unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) } == 1 {
        Ok(())
    } else {
        Err(format!("SendInput failed for key {code}"))
    }
}

pub(crate) fn run_shortcut(events: &[(u16, i32)]) {
    for (code, value) in events {
        if let Err(error) = run_held_key(*code, *value == 1) {
            eprintln!("{error}");
            break;
        }
    }
}

pub(crate) fn named_key_code(name: &str) -> Option<u16> {
    match name.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(0xa2),
        "alt" => Some(0xa4),
        "shift" => Some(0xa0),
        "super" | "win" | "windows" | "command" | "cmd" => Some(0x5b),
        "space" => Some(0x20),
        "enter" | "return" => Some(0x0d),
        "esc" | "escape" => Some(0x1b),
        "tab" => Some(0x09),
        "backspace" => Some(0x08),
        name if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() => {
            Some(u16::from(name.as_bytes()[0].to_ascii_uppercase()))
        }
        name if name
            .strip_prefix('f')
            .and_then(|number| number.parse::<u16>().ok())
            .is_some_and(|number| (1..=12).contains(&number)) =>
        {
            Some(0x6f + name[1..].parse::<u16>().expect("validated function key"))
        }
        _ => name.trim().parse().ok(),
    }
}
