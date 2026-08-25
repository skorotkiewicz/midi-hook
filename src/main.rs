mod named_key_code;

use crossterm::event::{self, Event, KeyCode as TerminalKeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use evdev::{Device, EventSummary, KeyCode, enumerate};
use midir::{Ignore, MidiInput};
use named_key_code::parse_shortcut;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

#[derive(Debug, PartialEq)]
enum Action {
    Command(String),
    HeldKey(u16),
    Shortcut(Vec<(u16, i32)>),
}

#[derive(Default)]
struct Config {
    device: Option<String>,
    actions: HashMap<u8, Action>,
}

struct RawMode;

impl RawMode {
    fn enable() -> Result<Self, String> {
        enable_raw_mode()
            .map_err(|error| format!("could not enable raw terminal mode: {error}"))?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

#[derive(Default)]
struct Capture {
    active: HashSet<u16>,
    events: Vec<(u16, i32)>,
    started: bool,
}

impl Capture {
    fn push(&mut self, code: u16, value: i32) -> bool {
        match value {
            1 if self.active.insert(code) => {
                self.started = true;
                self.events.push((code, 1));
            }
            0 if self.active.remove(&code) => self.events.push((code, 0)),
            _ => {}
        }
        self.started && self.active.is_empty()
    }
}

fn parse_config(text: &str) -> Result<Config, String> {
    let mut config = Config::default();
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {line_number}: expected KEY = VALUE"))?;
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("line {line_number}: value is empty"));
        }
        if key == "device" {
            if config.device.replace(value.to_owned()).is_some() {
                return Err(format!("line {line_number}: device is already set"));
            }
            continue;
        }
        let note = key
            .parse::<u8>()
            .ok()
            .filter(|note| *note <= 127)
            .ok_or_else(|| format!("line {line_number}: note must be between 0 and 127"))?;
        let action = if let Some(command) = value.strip_prefix("command ") {
            if command.trim().is_empty() {
                return Err(format!("line {line_number}: command is empty"));
            }
            Action::Command(command.to_owned())
        } else if let Some(code) = value.strip_prefix("key ") {
            Action::HeldKey(
                code.parse::<u16>()
                    .map_err(|_| format!("line {line_number}: invalid key code: {code}"))?,
            )
        } else if let Some(events) = value.strip_prefix("shortcut ") {
            Action::Shortcut(parse_shortcut(events, line_number)?)
        } else {
            return Err(format!(
                "line {line_number}: action must start with command, key, or shortcut"
            ));
        };
        if config.actions.insert(note, action).is_some() {
            return Err(format!("line {line_number}: note {note} is already mapped"));
        }
    }
    Ok(config)
}

fn update_config(text: &str, device: &str, note: u8, action: &str) -> Result<String, String> {
    parse_config(text)?;
    let mut found_device = false;
    let mut found_note = false;
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let key = raw_line
            .split_once('=')
            .map(|(key, _)| key.trim())
            .unwrap_or("");
        if key == "device" {
            lines.push(format!("device = {device}"));
            found_device = true;
        } else if key.parse::<u8>().ok() == Some(note) {
            lines.push(format!("{note} = {action}"));
            found_note = true;
        } else {
            lines.push(raw_line.to_owned());
        }
    }
    if !found_device {
        lines.push(format!("device = {device}"));
    }
    if !found_note {
        lines.push(format!("{note} = {action}"));
    }
    Ok(lines.join("\n") + "\n")
}

fn midi_note(message: &[u8]) -> Option<(u8, bool)> {
    if message.len() < 3 || message[1] > 127 || message[2] > 127 {
        return None;
    }
    match (message[0] & 0xf0, message[2]) {
        (0x90, 1..=127) => Some((message[1], true)),
        (0x80, _) | (0x90, 0) => Some((message[1], false)),
        _ => None,
    }
}

fn prompt(message: &str) -> Result<String, String> {
    print!("{message}");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut value = String::new();
    if io::stdin()
        .read_line(&mut value)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("input closed".into());
    }
    Ok(value.trim().to_owned())
}

fn choose_midi_port(
    input: &MidiInput,
    requested: Option<&str>,
    preferred: Option<&str>,
) -> Result<usize, String> {
    let ports = input.ports();
    if ports.is_empty() {
        return Err("no MIDI input ports found".into());
    }
    let names: Vec<_> = ports
        .iter()
        .map(|port| {
            input
                .port_name(port)
                .unwrap_or_else(|_| "unknown device".into())
        })
        .collect();
    if let Some(index) = requested {
        return index
            .parse::<usize>()
            .ok()
            .filter(|index| *index < ports.len())
            .ok_or_else(|| format!("port index must be between 0 and {}", ports.len() - 1));
    }
    if let Some(index) =
        preferred.and_then(|name| names.iter().position(|candidate| candidate == name))
    {
        println!("Using MIDI input {index}: {}", names[index]);
        return Ok(index);
    }
    println!("MIDI inputs:");
    for (index, name) in names.iter().enumerate() {
        println!("  {index}: {name}");
    }
    loop {
        let selection = prompt("Select MIDI input: ")?;
        if let Some(index) = selection
            .parse::<usize>()
            .ok()
            .filter(|index| *index < ports.len())
        {
            return Ok(index);
        }
        eprintln!("Enter a number between 0 and {}", ports.len() - 1);
    }
}

fn choose_keyboard() -> Result<PathBuf, String> {
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

fn capture_keyboard_action(path: &Path) -> Result<Option<String>, String> {
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
                        let pressed: HashSet<_> = capture
                            .events
                            .iter()
                            .filter_map(|(code, value)| (*value == 1).then_some(*code))
                            .collect();
                        if pressed.len() == 1 {
                            let code = pressed.into_iter().next().unwrap();
                            return Ok(Some(format!("key {code}")));
                        }
                        let events = capture
                            .events
                            .iter()
                            .map(|(code, value)| format!("{code}:{value}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        return Ok(Some(format!("shortcut {events}")));
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.to_string()),
        }
        if event::poll(Duration::from_millis(20)).map_err(|error| error.to_string())?
            && matches!(
                event::read().map_err(|error| error.to_string())?,
                Event::Key(key)
                    if key.kind == KeyEventKind::Press && key.code == TerminalKeyCode::Esc
            )
        {
            return Ok(None);
        }
    }
}

fn read_config(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

fn setup(path: &Path) -> Result<(), String> {
    let mut input = MidiInput::new("midi-hook-setup").map_err(|error| error.to_string())?;
    input.ignore(Ignore::None);
    let port_index = choose_midi_port(&input, None, None)?;
    let keyboard = choose_keyboard()?;
    let ports = input.ports();
    let port = ports.get(port_index).ok_or("MIDI input disappeared")?;
    let midi_device = input
        .port_name(port)
        .unwrap_or_else(|_| "unknown device".into());
    let (sender, receiver) = mpsc::sync_channel(1);
    let _connection = input
        .connect(
            port,
            "midi-hook-setup",
            move |_, message, _| {
                if let Some((note, true)) = midi_note(message) {
                    let _ = sender.try_send(note);
                }
            },
            (),
        )
        .map_err(|error| error.to_string())?;
    let mut text = read_config(path)?;
    println!("Setup ready. Press Ctrl+C while waiting for a MIDI note to quit.");
    loop {
        while receiver.try_recv().is_ok() {}
        println!("Press the MIDI note to map...");
        let note = receiver.recv().map_err(|_| "MIDI connection stopped")?;
        println!("Learned MIDI note {note}.");
        let action = loop {
            match prompt("Action [s=press shortcut, t=type shortcut, c=type command]: ")?.as_str() {
                "s" | "shortcut" => {
                    if let Some(shortcut) = capture_keyboard_action(&keyboard)? {
                        break shortcut;
                    }
                }
                "t" | "type" => {
                    let shortcut = prompt("Shortcut (example: ctrl+space+f4+c): ")?;
                    if parse_shortcut(&shortcut, 1).is_ok() {
                        break format!("shortcut {shortcut}");
                    }
                    eprintln!("Invalid shortcut; join at least two key names with +");
                }
                "c" | "command" => {
                    let command = prompt("Command: ")?;
                    if !command.is_empty() {
                        break format!("command {command}");
                    }
                    eprintln!("Command cannot be empty");
                }
                _ => eprintln!("Enter s, t, or c"),
            }
        };
        text = update_config(&text, &midi_device, note, &action)?;
        fs::write(path, &text)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        println!("Captured: {action}");
        println!("Saved {}.", path.display());
    }
}

fn shell_command(command: &str) -> Command {
    let (program, flag) = if cfg!(windows) {
        ("cmd.exe", "/C")
    } else {
        ("/bin/sh", "-c")
    };
    let mut process = Command::new(program);
    process.arg(flag).arg(command).stdin(Stdio::null());
    process
}

fn run_command(command: String) {
    thread::spawn(move || match shell_command(&command).status() {
        Ok(status) if !status.success() => eprintln!("command failed ({status}): {command}"),
        Err(error) => eprintln!("could not run command: {error}"),
        _ => {}
    });
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

fn run_held_key(code: u16, pressed: bool) -> Result<(), String> {
    run_ydotool(vec![format!("{code}:{}", u8::from(pressed))])
}

fn run_shortcut(events: &[(u16, i32)]) {
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

fn listen(config: Config, requested_port: Option<&str>) -> Result<(), String> {
    if config.actions.is_empty() {
        return Err("config contains no mappings; run `midi-hook setup`".into());
    }
    let mut input = MidiInput::new("midi-hook").map_err(|error| error.to_string())?;
    input.ignore(Ignore::None);
    let port_index = choose_midi_port(&input, requested_port, config.device.as_deref())?;
    let ports = input.ports();
    let port = ports.get(port_index).ok_or("MIDI input disappeared")?;
    let port_name = input
        .port_name(port)
        .unwrap_or_else(|_| "unknown device".into());
    let actions = config.actions;
    let held = Arc::new(Mutex::new(HashSet::new()));
    let callback_held = Arc::clone(&held);
    let connection = input
        .connect(
            port,
            "midi-hook",
            move |_, message, _| {
                let Some((note, pressed)) = midi_note(message) else {
                    return;
                };
                match actions.get(&note) {
                    Some(Action::Command(command)) if pressed => {
                        println!("note {note}: command {command}");
                        run_command(command.clone());
                    }
                    Some(Action::HeldKey(code)) => {
                        let Ok(mut held) = callback_held.lock() else {
                            return;
                        };
                        let changed = if pressed {
                            held.insert(*code)
                        } else {
                            held.remove(code)
                        };
                        if changed {
                            println!(
                                "note {note}: key {code} {}",
                                if pressed { "down" } else { "up" }
                            );
                            if let Err(error) = run_held_key(*code, pressed) {
                                eprintln!("{error}");
                            }
                        }
                    }
                    Some(Action::Shortcut(events)) if pressed => {
                        println!("note {note}: shortcut");
                        run_shortcut(events);
                    }
                    None if pressed => println!("note {note}: unmapped"),
                    _ => {}
                }
            },
            (),
        )
        .map_err(|error| error.to_string())?;
    println!("Listening on {port_name}. Press Enter to quit.");
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    drop(connection);
    if let Ok(mut held) = held.lock() {
        for code in held.drain() {
            let _ = run_held_key(code, false);
        }
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let first = args.next().ok_or(
        "usage: midi-hook setup [commands.conf]\n       midi-hook <commands.conf> [port-index]",
    )?;
    if matches!(first.as_str(), "setup" | "--setup") {
        let path = args.next().unwrap_or_else(|| "commands.conf".into());
        if args.next().is_some() {
            return Err("usage: midi-hook setup [commands.conf]".into());
        }
        return setup(Path::new(&path));
    }
    let requested_port = args.next();
    if args.next().is_some() {
        return Err("usage: midi-hook <commands.conf> [port-index]".into());
    }
    let text =
        fs::read_to_string(&first).map_err(|error| format!("could not read {first}: {error}"))?;
    listen(parse_config(&text)?, requested_port.as_deref())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_and_parses_all_actions() {
        let mut capture = Capture::default();
        assert!(!capture.push(57, 1));
        assert!(!capture.push(35, 1));
        assert!(!capture.push(35, 0));
        assert!(capture.push(57, 0));
        assert_eq!(capture.events, vec![(57, 1), (35, 1), (35, 0), (57, 0)]);

        let config = parse_config(
            "device = test\n60 = shortcut 57:1 35:1 35:0 57:0\n61 = key 29\n62 = command echo hi",
        )
        .unwrap();
        assert_eq!(
            config.actions[&60],
            Action::Shortcut(vec![(57, 1), (35, 1), (35, 0), (57, 0)])
        );
        assert_eq!(config.actions[&61], Action::HeldKey(29));
        assert_eq!(config.actions[&62], Action::Command("echo hi".into()));
        assert_eq!(
            parse_config("60 = shortcut ctrl+space+f4+c")
                .unwrap()
                .actions[&60],
            Action::Shortcut(vec![
                (29, 1),
                (57, 1),
                (62, 1),
                (46, 1),
                (46, 0),
                (62, 0),
                (57, 0),
                (29, 0),
            ])
        );
        assert_eq!(midi_note(&[0x90, 60, 100]), Some((60, true)));
        assert_eq!(midi_note(&[0x90, 60, 0]), Some((60, false)));
        assert!(parse_config("60 = shortcut 57:1").is_err());
    }
}
