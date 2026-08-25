use super::{Capture, RawMode, terminal_escape_pressed};
use crate::prompt;
use evdev::{Device, EventSummary, KeyCode, enumerate};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

pub(crate) type Keyboard = PathBuf;

pub(crate) fn choose_keyboard() -> Result<Keyboard, String> {
    let keyboards: Vec<_> = enumerate()
        .filter(|(_, device)| {
            !device
                .name()
                .is_some_and(|name| name.to_ascii_lowercase().contains("ydotool"))
                && device.supported_keys().is_some_and(|keys| {
                    keys.contains(KeyCode::KEY_A)
                        && keys.contains(KeyCode::KEY_ENTER)
                        && keys.contains(KeyCode::KEY_SPACE)
                })
        })
        .map(|(path, device)| {
            let name = device.name().unwrap_or("unknown keyboard").to_owned();
            (path, name)
        })
        .collect();
    if keyboards.is_empty() {
        return Err(
            "no readable keyboards found in /dev/input; check input-group permissions".into(),
        );
    }
    println!("Keyboards:");
    for (index, (path, name)) in keyboards.iter().enumerate() {
        println!("  {index}: {name} ({})", path.display());
    }
    loop {
        let selection = prompt("Select keyboard: ")?;
        if let Some((path, _)) = selection
            .parse::<usize>()
            .ok()
            .and_then(|index| keyboards.get(index))
        {
            return Ok(path.clone());
        }
        eprintln!("Enter a number between 0 and {}", keyboards.len() - 1);
    }
}

pub(crate) fn capture_keyboard_action(path: &Path) -> Result<Option<String>, String> {
    let mut device = Device::open(path)
        .map_err(|error| format!("could not open keyboard {}: {error}", path.display()))?;
    device
        .set_nonblocking(true)
        .map_err(|error| format!("could not poll keyboard {}: {error}", path.display()))?;
    println!("Hold the shortcut keys, then release them all. Press Esc to cancel.");
    let _raw_mode = RawMode::enable()?;
    let mut capture = Capture::default();
    loop {
        match device.fetch_events() {
            Ok(events) => {
                for input_event in events {
                    if let EventSummary::Key(_, key, value) = input_event.destructure()
                        && capture.push(key.code(), value)
                    {
                        return Ok(Some(capture.action()));
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.to_string()),
        }
        if terminal_escape_pressed()? {
            return Ok(None);
        }
        thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn run_ydotool(events: Vec<String>) -> Result<(), String> {
    match Command::new("ydotool")
        .arg("key")
        .args(&events)
        .stdin(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "ydotool failed ({status}): key {}",
            events.join(" ")
        )),
        Err(error) => Err(format!("could not run ydotool: {error}")),
    }
}

pub(crate) fn run_held_key(code: u16, pressed: bool) -> Result<(), String> {
    run_ydotool(vec![format!("{code}:{}", u8::from(pressed))])
}

pub(crate) fn run_shortcut(events: &[(u16, i32)]) {
    let events = events
        .iter()
        .map(|(code, value)| format!("{code}:{value}"))
        .collect();
    thread::spawn(move || {
        if let Err(error) = run_ydotool(events) {
            eprintln!("{error}");
        }
    });
}

pub(crate) fn named_key_code(name: &str) -> Option<u16> {
    let key = match name.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => KeyCode::KEY_LEFTCTRL,
        "alt" => KeyCode::KEY_LEFTALT,
        "shift" => KeyCode::KEY_LEFTSHIFT,
        "super" | "win" | "windows" | "command" | "cmd" => KeyCode::KEY_LEFTMETA,
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
