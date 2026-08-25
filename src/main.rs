mod keyboard;
mod named_key_code;

use keyboard::{capture_keyboard_action, choose_keyboard, run_held_key, run_shortcut};
use midir::{Ignore, MidiInput};
use named_key_code::parse_shortcut;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

#[derive(Debug, PartialEq)]
enum Action {
    Command(String),
    HeldKey(u16),
    Shortcut(Vec<(u16, i32)>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MidiTrigger {
    Chord(Vec<u8>),
    Sequence(Vec<u8>),
}

#[derive(Default)]
struct Config {
    device: Option<String>,
    actions: HashMap<MidiTrigger, Action>,
}

fn parse_midi_trigger(value: &str, line: usize) -> Result<MidiTrigger, String> {
    if value.contains('+') && value.contains('>') {
        return Err(format!("line {line}: MIDI trigger cannot mix + and >"));
    }
    let is_sequence = value.contains('>');
    let separator = if is_sequence { '>' } else { '+' };
    let mut notes = Vec::new();
    for value in value.split(separator) {
        let note = value
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|note| *note <= 127)
            .ok_or_else(|| format!("line {line}: MIDI note must be between 0 and 127"))?;
        notes.push(note);
    }
    if notes.is_empty() {
        return Err(format!("line {line}: MIDI trigger is empty"));
    }
    if is_sequence {
        Ok(MidiTrigger::Sequence(notes))
    } else {
        notes.sort_unstable();
        if notes.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(format!("line {line}: MIDI chord contains duplicate notes"));
        }
        Ok(MidiTrigger::Chord(notes))
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
        let trigger = parse_midi_trigger(key, line_number)?;
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
        if matches!(trigger, MidiTrigger::Sequence(_)) && matches!(action, Action::HeldKey(_)) {
            return Err(format!(
                "line {line_number}: ordered sequences cannot hold a key"
            ));
        }
        if config.actions.insert(trigger.clone(), action).is_some() {
            return Err(format!(
                "line {line_number}: MIDI trigger {trigger:?} is already mapped"
            ));
        }
    }
    Ok(config)
}

fn update_config(
    text: &str,
    device: &str,
    trigger: &MidiTrigger,
    action: &str,
) -> Result<String, String> {
    parse_config(text)?;
    let trigger_text = trigger_label(trigger);
    let mut found_device = false;
    let mut found_trigger = false;
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let key = raw_line
            .split_once('=')
            .map(|(key, _)| key.trim())
            .unwrap_or("");
        if key == "device" {
            lines.push(format!("device = {device}"));
            found_device = true;
        } else if parse_midi_trigger(key, 1).ok().as_ref() == Some(trigger) {
            lines.push(format!("{trigger_text} = {action}"));
            found_trigger = true;
        } else {
            lines.push(raw_line.to_owned());
        }
    }
    if !found_device {
        lines.push(format!("device = {device}"));
    }
    if !found_trigger {
        lines.push(format!("{trigger_text} = {action}"));
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

fn capture_midi_trigger(receiver: &mpsc::Receiver<(u8, bool)>) -> Result<Vec<u8>, String> {
    let mut active = HashSet::new();
    let mut captured = Vec::new();
    let mut seen = HashSet::new();
    loop {
        let (note, pressed) = receiver.recv().map_err(|_| "MIDI connection stopped")?;
        if pressed {
            active.insert(note);
            if seen.insert(note) {
                captured.push(note);
            }
        } else {
            active.remove(&note);
        }
        if !captured.is_empty() && active.is_empty() {
            return Ok(captured);
        }
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
    let (sender, receiver) = mpsc::channel();
    let _connection = input
        .connect(
            port,
            "midi-hook-setup",
            move |_, message, _| {
                if let Some(event) = midi_note(message) {
                    let _ = sender.send(event);
                }
            },
            (),
        )
        .map_err(|error| error.to_string())?;
    let mut text = read_config(path)?;
    println!("Setup ready. Press Ctrl+C while waiting for a MIDI note to quit.");
    loop {
        while receiver.try_recv().is_ok() {}
        println!("Hold the MIDI notes to map, then release them all...");
        let captured = capture_midi_trigger(&receiver)?;
        let trigger = if captured.len() == 1 {
            MidiTrigger::Chord(captured)
        } else {
            loop {
                match prompt("MIDI trigger [c=unordered chord, o=ordered sequence]: ")?.as_str() {
                    "c" | "chord" => {
                        let mut notes = captured.clone();
                        notes.sort_unstable();
                        break MidiTrigger::Chord(notes);
                    }
                    "o" | "ordered" | "sequence" => {
                        break MidiTrigger::Sequence(captured.clone());
                    }
                    _ => eprintln!("Enter c or o"),
                }
            }
        };
        println!("Learned MIDI trigger {}.", trigger_label(&trigger));
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
        text = update_config(&text, &midi_device, &trigger, &action)?;
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

fn chord_active(notes: &[u8], held: &HashSet<u8>) -> bool {
    notes.iter().all(|note| held.contains(note))
}

fn advance_sequence(notes: &[u8], position: &mut usize, note: u8) -> bool {
    if note == notes[*position] {
        *position += 1;
    } else {
        *position = usize::from(note == notes[0]);
    }
    if *position == notes.len() {
        *position = 0;
        true
    } else {
        false
    }
}

fn trigger_notes(trigger: &MidiTrigger) -> &[u8] {
    match trigger {
        MidiTrigger::Chord(notes) | MidiTrigger::Sequence(notes) => notes,
    }
}

fn trigger_label(trigger: &MidiTrigger) -> String {
    let (notes, separator) = match trigger {
        MidiTrigger::Chord(notes) => (notes, "+"),
        MidiTrigger::Sequence(notes) => (notes, ">"),
    };
    notes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(separator)
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
    let held_keys = Arc::new(Mutex::new(HashSet::new()));
    let callback_held_keys = Arc::clone(&held_keys);
    let mut held_notes = HashSet::new();
    let mut active_triggers = HashSet::new();
    let mut sequence_positions = HashMap::new();
    let connection = input
        .connect(
            port,
            "midi-hook",
            move |_, message, _| {
                let Some((note, pressed)) = midi_note(message) else {
                    return;
                };
                if pressed {
                    held_notes.insert(note);
                } else {
                    held_notes.remove(&note);
                }

                if pressed {
                    for (trigger, action) in &actions {
                        let MidiTrigger::Sequence(notes) = trigger else {
                            continue;
                        };
                        let position = sequence_positions.entry(trigger.clone()).or_insert(0);
                        if advance_sequence(notes, position, note) {
                            let label = trigger_label(trigger);
                            match action {
                                Action::Command(command) => {
                                    println!("notes {label}: command {command}");
                                    run_command(command.clone());
                                }
                                Action::Shortcut(events) => {
                                    println!("notes {label}: shortcut");
                                    run_shortcut(events);
                                }
                                Action::HeldKey(_) => unreachable!(),
                            }
                        }
                    }
                }

                for (trigger, action) in &actions {
                    let MidiTrigger::Chord(notes) = trigger else {
                        continue;
                    };
                    let now_active = chord_active(notes, &held_notes);
                    let was_active = active_triggers.contains(trigger);
                    if now_active == was_active {
                        continue;
                    }
                    let label = trigger_label(trigger);
                    if now_active {
                        active_triggers.insert(trigger.clone());
                        match action {
                            Action::Command(command) => {
                                println!("notes {label}: command {command}");
                                run_command(command.clone());
                            }
                            Action::HeldKey(code) => {
                                let Ok(mut held) = callback_held_keys.lock() else {
                                    return;
                                };
                                if held.insert(*code) {
                                    println!("notes {label}: key {code} down");
                                    if let Err(error) = run_held_key(*code, true) {
                                        eprintln!("{error}");
                                    }
                                }
                            }
                            Action::Shortcut(events) => {
                                println!("notes {label}: shortcut");
                                run_shortcut(events);
                            }
                        }
                    } else {
                        active_triggers.remove(trigger);
                        if let Action::HeldKey(code) = action {
                            let Ok(mut held) = callback_held_keys.lock() else {
                                return;
                            };
                            if held.remove(code) {
                                println!("notes {label}: key {code} up");
                                if let Err(error) = run_held_key(*code, false) {
                                    eprintln!("{error}");
                                }
                            }
                        }
                    }
                }

                if pressed
                    && !actions
                        .keys()
                        .any(|trigger| trigger_notes(trigger).contains(&note))
                {
                    println!("note {note}: unmapped");
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
    if let Ok(mut held) = held_keys.lock() {
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
        let mut capture = keyboard::Capture::default();
        assert!(!capture.push(57, 1));
        assert!(!capture.push(35, 1));
        assert!(!capture.push(35, 0));
        assert!(capture.push(57, 0));
        assert_eq!(capture.events, vec![(57, 1), (35, 1), (35, 0), (57, 0)]);
        assert_eq!(capture.action(), "shortcut 57:1 35:1 35:0 57:0");

        let config = parse_config(
            "device = test\n60 = shortcut 57:1 35:1 35:0 57:0\n61 = key 29\n62 = command echo hi\n62+60+61 = command combo\n48>50>52 = command sequence",
        )
        .unwrap();
        assert_eq!(
            config.actions[&MidiTrigger::Chord(vec![60])],
            Action::Shortcut(vec![(57, 1), (35, 1), (35, 0), (57, 0)])
        );
        assert_eq!(
            config.actions[&MidiTrigger::Chord(vec![61])],
            Action::HeldKey(29)
        );
        assert_eq!(
            config.actions[&MidiTrigger::Chord(vec![62])],
            Action::Command("echo hi".into())
        );
        assert_eq!(
            config.actions[&MidiTrigger::Chord(vec![60, 61, 62])],
            Action::Command("combo".into())
        );
        assert_eq!(
            config.actions[&MidiTrigger::Sequence(vec![48, 50, 52])],
            Action::Command("sequence".into())
        );
        let codes: Vec<_> = ["ctrl", "space", "f4", "c"]
            .into_iter()
            .map(|name| keyboard::named_key_code(name).unwrap())
            .collect();
        let mut expected: Vec<_> = codes.iter().map(|code| (*code, 1)).collect();
        expected.extend(codes.iter().rev().map(|code| (*code, 0)));
        assert_eq!(
            parse_config("60 = shortcut ctrl+space+f4+c")
                .unwrap()
                .actions[&MidiTrigger::Chord(vec![60])],
            Action::Shortcut(expected)
        );
        let held = HashSet::from([60, 61, 62]);
        assert!(chord_active(&[60, 61, 62], &held));
        assert!(!chord_active(&[60, 61, 63], &held));
        let mut position = 0;
        assert!(!advance_sequence(&[48, 50, 52], &mut position, 48));
        assert!(!advance_sequence(&[48, 50, 52], &mut position, 49));
        assert_eq!(position, 0);
        assert!(!advance_sequence(&[48, 50, 52], &mut position, 48));
        assert!(!advance_sequence(&[48, 50, 52], &mut position, 50));
        assert!(advance_sequence(&[48, 50, 52], &mut position, 52));

        let (sender, receiver) = mpsc::channel();
        for event in [(61, true), (60, true), (60, false), (61, false)] {
            sender.send(event).unwrap();
        }
        assert_eq!(capture_midi_trigger(&receiver).unwrap(), vec![61, 60]);
        let updated = update_config(
            "device = old\n61+60 = command old\n",
            "new",
            &MidiTrigger::Chord(vec![60, 61]),
            "command new",
        )
        .unwrap();
        let updated = parse_config(&updated).unwrap();
        assert_eq!(updated.device.as_deref(), Some("new"));
        assert_eq!(
            updated.actions[&MidiTrigger::Chord(vec![60, 61])],
            Action::Command("new".into())
        );
        assert_eq!(midi_note(&[0x90, 60, 100]), Some((60, true)));
        assert_eq!(midi_note(&[0x90, 60, 0]), Some((60, false)));
        assert!(parse_config("60 = shortcut 57:1").is_err());
    }
}
