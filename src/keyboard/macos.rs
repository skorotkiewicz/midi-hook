use super::{Capture, RawMode, terminal_escape_pressed};
use std::ffi::c_void;
use std::thread;
use std::time::Duration;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *const c_void;
    fn CGEventPost(location: u32, event: *const c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
}

pub(crate) struct Keyboard;

pub(crate) fn choose_keyboard() -> Result<Keyboard, String> {
    println!("Using the system keyboard. macOS may request Input Monitoring permission.");
    Ok(Keyboard)
}

fn is_pressed(code: u16) -> bool {
    unsafe { CGEventSourceKeyState(0, code) }
}

pub(crate) fn capture_keyboard_action(_: &Keyboard) -> Result<Option<String>, String> {
    println!("Hold the shortcut keys, then release them all. Press Esc to cancel.");
    let _raw_mode = RawMode::enable()?;
    let mut previous = [false; 128];
    let mut capture = Capture::default();
    loop {
        if is_pressed(53) || terminal_escape_pressed()? {
            return Ok(None);
        }
        for code in 0u16..128 {
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

pub(crate) fn prepare_output(_: &[u16]) -> Result<(), String> {
    Ok(())
}

pub(crate) fn run_held_key(code: u16, pressed: bool) -> Result<(), String> {
    let event = unsafe { CGEventCreateKeyboardEvent(std::ptr::null(), code, pressed) };
    if event.is_null() {
        return Err(format!(
            "could not create macOS keyboard event for key {code}"
        ));
    }
    unsafe {
        CGEventPost(0, event);
        CFRelease(event);
    }
    Ok(())
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
        "ctrl" | "control" => Some(59),
        "alt" => Some(58),
        "shift" => Some(56),
        "super" | "win" | "windows" | "command" | "cmd" => Some(55),
        "space" => Some(49),
        "enter" | "return" => Some(36),
        "esc" | "escape" => Some(53),
        "tab" => Some(48),
        "backspace" => Some(51),
        "a" => Some(0),
        "b" => Some(11),
        "c" => Some(8),
        "d" => Some(2),
        "e" => Some(14),
        "f" => Some(3),
        "g" => Some(5),
        "h" => Some(4),
        "i" => Some(34),
        "j" => Some(38),
        "k" => Some(40),
        "l" => Some(37),
        "m" => Some(46),
        "n" => Some(45),
        "o" => Some(31),
        "p" => Some(35),
        "q" => Some(12),
        "r" => Some(15),
        "s" => Some(1),
        "t" => Some(17),
        "u" => Some(32),
        "v" => Some(9),
        "w" => Some(13),
        "x" => Some(7),
        "y" => Some(16),
        "z" => Some(6),
        "f1" => Some(122),
        "f2" => Some(120),
        "f3" => Some(99),
        "f4" => Some(118),
        "f5" => Some(96),
        "f6" => Some(97),
        "f7" => Some(98),
        "f8" => Some(100),
        "f9" => Some(101),
        "f10" => Some(109),
        "f11" => Some(103),
        "f12" => Some(111),
        _ => name.trim().parse().ok(),
    }
}
