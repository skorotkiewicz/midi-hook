mod keyboard;
mod midi_feedback;
mod midi_monitor;
mod shortcut;

use keyboard::{
    capture_keyboard_action, choose_keyboard, prepare_output, run_held_key, run_shortcut,
};
use midi_feedback::{MidiFeedback, connect_midi_feedback};
use midi_monitor::test_midi;
use midir::{Ignore, MidiInput, MidiOutput};
use shortcut::parse_shortcut;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
enum Action {
    Command(String),
    Toggle(String),
    HeldKey(u16),
    Shortcut(Vec<(u16, i32)>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MidiTrigger {
    Chord(Vec<u8>),
    Sequence(Vec<u8>),
    Control(u8),
}

#[derive(Default)]
struct Config {
    device: Option<String>,
    output: Option<String>,
    actions: HashMap<MidiTrigger, Action>,
}

fn parse_midi_trigger(value: &str, line: usize) -> Result<MidiTrigger, String> {
    if let Some(control) = value.strip_prefix("cc ") {
        return control
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|control| *control <= 127)
            .map(MidiTrigger::Control)
            .ok_or_else(|| format!("line {line}: MIDI control must be between 0 and 127"));
    }
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
        if key == "output" {
            if config.output.replace(value.to_owned()).is_some() {
                return Err(format!("line {line_number}: output is already set"));
            }
            continue;
        }
        let trigger = parse_midi_trigger(key, line_number)?;
        let action = if let Some(command) = value.strip_prefix("command ") {
            if command.trim().is_empty() {
                return Err(format!("line {line_number}: command is empty"));
            }
            Action::Command(command.to_owned())
        } else if let Some(command) = value.strip_prefix("toggle ") {
            if command.trim().is_empty() {
                return Err(format!("line {line_number}: toggle command is empty"));
            }
            Action::Toggle(command.to_owned())
        } else if let Some(code) = value.strip_prefix("key ") {
            Action::HeldKey(
                code.parse::<u16>()
                    .map_err(|_| format!("line {line_number}: invalid key code: {code}"))?,
            )
        } else if let Some(events) = value.strip_prefix("shortcut ") {
            Action::Shortcut(parse_shortcut(events, line_number)?)
        } else {
            return Err(format!(
                "line {line_number}: action must start with command, toggle, key, or shortcut"
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
    output: Option<&str>,
    trigger: &MidiTrigger,
    action: &str,
) -> Result<String, String> {
    parse_config(text)?;
    let trigger_text = trigger_label(trigger);
    let mut found_device = false;
    let mut found_output = false;
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
        } else if key == "output" {
            if let Some(output) = output {
                lines.push(format!("output = {output}"));
                found_output = true;
            }
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
    if let Some(output) = output.filter(|_| !found_output) {
        lines.push(format!("output = {output}"));
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

fn midi_control(message: &[u8]) -> Option<(u8, u8)> {
    (message.len() >= 3 && message[1] <= 127 && message[2] <= 127 && message[0] & 0xf0 == 0xb0)
        .then_some((message[1], message[2]))
}

#[derive(Debug, PartialEq)]
enum MidiCapture {
    Notes(Vec<u8>),
    Control(u8),
}

#[derive(Clone, Copy)]
enum MidiInputEvent {
    Note(u8, bool),
    Control(u8, u8),
}

fn capture_midi_trigger(receiver: &mpsc::Receiver<MidiInputEvent>) -> Result<MidiCapture, String> {
    let mut active = HashSet::new();
    let mut captured = Vec::new();
    let mut seen = HashSet::new();
    loop {
        match receiver.recv().map_err(|_| "MIDI connection stopped")? {
            MidiInputEvent::Control(control, 1..=127) => {
                return Ok(MidiCapture::Control(control));
            }
            MidiInputEvent::Control(_, _) => {}
            MidiInputEvent::Note(note, pressed) => {
                if pressed {
                    active.insert(note);
                    if seen.insert(note) {
                        captured.push(note);
                    }
                } else {
                    active.remove(&note);
                }
                if !captured.is_empty() && active.is_empty() {
                    return Ok(MidiCapture::Notes(captured));
                }
            }
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

fn choose_midi_output(output: &MidiOutput) -> Result<Option<String>, String> {
    let ports = output.ports();
    if ports.is_empty() {
        println!("No MIDI outputs found; LED feedback disabled.");
        return Ok(None);
    }
    println!("MIDI outputs for LED feedback:");
    for (index, port) in ports.iter().enumerate() {
        let name = output
            .port_name(port)
            .unwrap_or_else(|_| "unknown device".into());
        println!("  {index}: {name}");
    }
    loop {
        let selection = prompt("Select MIDI output, or press Enter to disable: ")?;
        if selection.is_empty() {
            return Ok(None);
        }
        if let Some(port) = selection
            .parse::<usize>()
            .ok()
            .and_then(|index| ports.get(index))
        {
            return Ok(Some(
                output
                    .port_name(port)
                    .unwrap_or_else(|_| "unknown device".into()),
            ));
        }
        eprintln!(
            "Enter a number between 0 and {}, or leave blank",
            ports.len() - 1
        );
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
    ctrlc::set_handler(|| {
        let _ = crossterm::terminal::disable_raw_mode();
        std::process::exit(130);
    })
    .map_err(|error| format!("could not install Ctrl+C handler: {error}"))?;
    let mut input = MidiInput::new("midi-hook-setup").map_err(|error| error.to_string())?;
    input.ignore(Ignore::None);
    let port_index = choose_midi_port(&input, None, None)?;
    let output = MidiOutput::new("midi-hook-setup").map_err(|error| error.to_string())?;
    let midi_output = choose_midi_output(&output)?;
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
                let event = midi_note(message)
                    .map(|(note, pressed)| MidiInputEvent::Note(note, pressed))
                    .or_else(|| {
                        midi_control(message)
                            .map(|(control, value)| MidiInputEvent::Control(control, value))
                    });
                if let Some(event) = event {
                    let _ = sender.send(event);
                }
            },
            (),
        )
        .map_err(|error| error.to_string())?;
    let mut text = read_config(path)?;
    println!("Setup ready. Press Ctrl+C while waiting for MIDI input to quit.");
    loop {
        while receiver.try_recv().is_ok() {}
        println!("Use a MIDI control, or hold MIDI notes and release them all...");
        let trigger = match capture_midi_trigger(&receiver)? {
            MidiCapture::Control(control) => MidiTrigger::Control(control),
            MidiCapture::Notes(captured) if captured.len() == 1 => MidiTrigger::Chord(captured),
            MidiCapture::Notes(captured) => loop {
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
            },
        };
        println!("Learned MIDI trigger {}.", trigger_label(&trigger));
        let action = loop {
            match prompt(
                "Action [s=press shortcut, t=type shortcut, c=command, g=toggle command]: ",
            )?
            .as_str()
            {
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
                "g" | "toggle" => {
                    let command = prompt("Toggle command: ")?;
                    if !command.is_empty() {
                        break format!("toggle {command}");
                    }
                    eprintln!("Command cannot be empty");
                }
                _ => eprintln!("Enter s, t, c, or g"),
            }
        };
        text = update_config(
            &text,
            &midi_device,
            midi_output.as_deref(),
            &trigger,
            &action,
        )?;
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

fn toggle_command(running: &mut HashMap<MidiTrigger, Child>, trigger: &MidiTrigger, command: &str) {
    if let Some(mut child) = running.remove(trigger) {
        match child.try_wait() {
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                println!("stopped toggle: {command}");
                return;
            }
            Ok(Some(_)) => {}
            Err(error) => {
                eprintln!("could not check toggle command: {error}");
                return;
            }
        }
    }
    match shell_command(command).spawn() {
        Ok(child) => {
            running.insert(trigger.clone(), child);
            println!("started toggle: {command}");
        }
        Err(error) => eprintln!("could not start toggle command: {error}"),
    }
}

fn stop_toggle_commands(running: &mut HashMap<MidiTrigger, Child>) {
    for (_, mut child) in running.drain() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn expand_cc_command(command: &str, value: u8) -> Option<String> {
    let parameterized = command.contains("{value}") || command.contains("{percent}");
    if value == 0 && !parameterized {
        return None;
    }
    let percent = (u16::from(value) * 100 + 63) / 127;
    Some(
        command
            .replace("{value}", &value.to_string())
            .replace("{percent}", &percent.to_string()),
    )
}

fn chord_active(notes: &[u8], held: &HashSet<u8>) -> bool {
    notes.iter().all(|note| held.contains(note))
}

fn sequence_failure(notes: &[u8]) -> Vec<usize> {
    let mut failure = vec![0; notes.len()];
    let mut matched = 0;
    for index in 1..notes.len() {
        while matched > 0 && notes[index] != notes[matched] {
            matched = failure[matched - 1];
        }
        if notes[index] == notes[matched] {
            matched += 1;
        }
        failure[index] = matched;
    }
    failure
}

fn advance_sequence(notes: &[u8], failure: &[usize], position: &mut usize, note: u8) -> bool {
    while *position > 0 && note != notes[*position] {
        *position = failure[*position - 1];
    }
    if note == notes[*position] {
        *position += 1;
    }
    if *position == notes.len() {
        *position = failure[*position - 1];
        true
    } else {
        false
    }
}

fn sequence_note_set(notes: &[u8]) -> Vec<u8> {
    let mut notes = notes.to_vec();
    notes.sort_unstable();
    notes.dedup();
    notes
}

fn sequence_takes_priority(chord: &[u8], completed_sequences: &[Vec<u8>]) -> bool {
    completed_sequences.iter().any(|sequence| sequence == chord)
}

fn trigger_notes(trigger: &MidiTrigger) -> &[u8] {
    match trigger {
        MidiTrigger::Chord(notes) | MidiTrigger::Sequence(notes) => notes,
        MidiTrigger::Control(_) => &[],
    }
}

fn note_numbers(notes: &[u8], separator: &str) -> String {
    notes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(separator)
}

fn trigger_label(trigger: &MidiTrigger) -> String {
    match trigger {
        MidiTrigger::Chord(notes) => note_numbers(notes, "+"),
        MidiTrigger::Sequence(notes) => note_numbers(notes, ">"),
        MidiTrigger::Control(control) => format!("cc {control}"),
    }
}

fn change_held_key(held_keys: &Arc<Mutex<HashSet<u16>>>, code: u16, pressed: bool) -> Option<bool> {
    held_keys.lock().ok().map(|mut held| {
        if pressed {
            held.insert(code)
        } else {
            held.remove(&code)
        }
    })
}

fn set_midi_feedback(
    feedback: &Option<Arc<Mutex<MidiFeedback>>>,
    trigger: &MidiTrigger,
    pressed: bool,
) {
    if let MidiTrigger::Chord(notes) = trigger
        && let Some(feedback) = feedback
        && let Ok(mut feedback) = feedback.lock()
    {
        feedback.set_notes(notes, pressed);
    }
}

fn listen(config: Config, requested_port: Option<&str>) -> Result<(), String> {
    if config.actions.is_empty() {
        return Err("config contains no mappings; run `midi-hook setup`".into());
    }
    let feedback = connect_midi_feedback(config.output.as_deref())?
        .map(|feedback| Arc::new(Mutex::new(feedback)));
    let mut output_codes = HashSet::new();
    for action in config.actions.values() {
        match action {
            Action::HeldKey(code) => {
                output_codes.insert(*code);
            }
            Action::Shortcut(events) => output_codes.extend(events.iter().map(|(code, _)| *code)),
            Action::Command(_) | Action::Toggle(_) => {}
        }
    }
    if !output_codes.is_empty() {
        prepare_output(&output_codes.into_iter().collect::<Vec<_>>())?;
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
    let sequence_failures: HashMap<_, _> = actions
        .keys()
        .filter_map(|trigger| match trigger {
            MidiTrigger::Sequence(notes) => Some((trigger.clone(), sequence_failure(notes))),
            _ => None,
        })
        .collect();
    let held_keys = Arc::new(Mutex::new(HashSet::new()));
    let cleanup_held_keys = Arc::clone(&held_keys);
    let cleanup_feedback = feedback.clone();
    ctrlc::set_handler(move || {
        if let Ok(mut held) = cleanup_held_keys.lock() {
            for code in held.drain() {
                let _ = run_held_key(code, false);
            }
        }
        if let Some(feedback) = &cleanup_feedback
            && let Ok(mut feedback) = feedback.lock()
        {
            feedback.release_all();
        }
        let _ = crossterm::terminal::disable_raw_mode();
        std::process::exit(130);
    })
    .map_err(|error| format!("could not install Ctrl+C handler: {error}"))?;
    let callback_held_keys = Arc::clone(&held_keys);
    let callback_feedback = feedback.clone();
    let (midi_sender, midi_receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut held_notes = HashSet::new();
        let mut active_triggers = HashSet::new();
        let mut active_controls = HashSet::new();
        let mut sequence_positions = HashMap::new();
        let mut running_toggles = HashMap::new();
        let mut last_note = HashMap::new();
        'events: while let Ok(event) = midi_receiver.recv() {
            if let MidiInputEvent::Control(control, value) = event {
                let trigger = MidiTrigger::Control(control);
                let Some(action) = actions.get(&trigger) else {
                    if value > 0 {
                        println!("cc {control} value {value}: unmapped");
                    }
                    continue 'events;
                };
                if let Action::Command(command) = action {
                    if let Some(command) = expand_cc_command(command, value) {
                        println!("cc {control} value {value}: command {command}");
                        run_command(command);
                    }
                    continue 'events;
                }
                let now_active = value > 0;
                let was_active = active_controls.contains(&control);
                if now_active == was_active {
                    continue 'events;
                }
                if now_active {
                    active_controls.insert(control);
                    match action {
                        Action::HeldKey(code) => {
                            if change_held_key(&callback_held_keys, *code, true) == Some(true) {
                                println!("cc {control}: key {code} down");
                                if let Err(error) = run_held_key(*code, true) {
                                    eprintln!("{error}");
                                }
                            }
                        }
                        Action::Shortcut(events) => {
                            println!("cc {control}: shortcut");
                            run_shortcut(events);
                        }
                        Action::Toggle(command) => {
                            toggle_command(&mut running_toggles, &trigger, command);
                        }
                        Action::Command(_) => unreachable!(),
                    }
                } else {
                    active_controls.remove(&control);
                    if let Action::HeldKey(code) = action
                        && change_held_key(&callback_held_keys, *code, false) == Some(true)
                    {
                        println!("cc {control}: key {code} up");
                        if let Err(error) = run_held_key(*code, false) {
                            eprintln!("{error}");
                        }
                    }
                }
                continue 'events;
            }
            let MidiInputEvent::Note(note, pressed) = event else {
                continue 'events;
            };
            if pressed {
                let now = Instant::now();
                if last_note
                    .insert(note, now)
                    .is_some_and(|last| now.duration_since(last) < Duration::from_millis(15))
                {
                    continue 'events;
                }
                held_notes.insert(note);
            } else {
                held_notes.remove(&note);
            }

            let mut completed_sequences = Vec::new();
            if pressed {
                for (trigger, action) in &actions {
                    let MidiTrigger::Sequence(notes) = trigger else {
                        continue;
                    };
                    let position = sequence_positions.entry(trigger.clone()).or_insert(0);
                    if advance_sequence(notes, &sequence_failures[trigger], position, note) {
                        completed_sequences.push(sequence_note_set(notes));
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
                            Action::Toggle(command) => {
                                toggle_command(&mut running_toggles, trigger, command);
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
                    if sequence_takes_priority(notes, &completed_sequences) {
                        continue;
                    }
                    set_midi_feedback(&callback_feedback, trigger, true);
                    match action {
                        Action::Command(command) => {
                            println!("notes {label}: command {command}");
                            run_command(command.clone());
                        }
                        Action::HeldKey(code) => {
                            if change_held_key(&callback_held_keys, *code, true) == Some(true) {
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
                        Action::Toggle(command) => {
                            toggle_command(&mut running_toggles, trigger, command);
                        }
                    }
                } else {
                    active_triggers.remove(trigger);
                    set_midi_feedback(&callback_feedback, trigger, false);
                    if let Action::HeldKey(code) = action
                        && change_held_key(&callback_held_keys, *code, false) == Some(true)
                    {
                        println!("notes {label}: key {code} up");
                        if let Err(error) = run_held_key(*code, false) {
                            eprintln!("{error}");
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
        }
        stop_toggle_commands(&mut running_toggles);
    });
    let connection = input
        .connect(
            port,
            "midi-hook",
            move |_, message, _| {
                let event = midi_note(message)
                    .map(|(note, pressed)| MidiInputEvent::Note(note, pressed))
                    .or_else(|| {
                        midi_control(message)
                            .map(|(control, value)| MidiInputEvent::Control(control, value))
                    });
                if let Some(event) = event {
                    let _ = midi_sender.send(event);
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
    worker
        .join()
        .map_err(|_| "MIDI worker stopped unexpectedly".to_owned())?;
    if let Ok(mut held) = held_keys.lock() {
        for code in held.drain() {
            let _ = run_held_key(code, false);
        }
    }
    if let Some(feedback) = feedback
        && let Ok(mut feedback) = feedback.lock()
    {
        feedback.release_all();
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let first = args.next().ok_or(
        "usage: midi-hook setup [commands.conf]\n       midi-hook test [--details] [port-index]\n       midi-hook --version\n       midi-hook <commands.conf> [port-index]",
    )?;
    if matches!(first.as_str(), "--version" | "-V") {
        if args.next().is_some() {
            return Err("usage: midi-hook --version".into());
        }
        println!(
            "midi-hook {} 🎹 no wrong notes, only unmapped ones",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    if matches!(first.as_str(), "setup" | "--setup") {
        let path = args.next().unwrap_or_else(|| "commands.conf".into());
        if args.next().is_some() {
            return Err("usage: midi-hook setup [commands.conf]".into());
        }
        return setup(Path::new(&path));
    }
    if first == "test" {
        let mut requested_port = args.next();
        let details = requested_port.as_deref() == Some("--details");
        if details {
            requested_port = args.next();
        }
        if args.next().is_some() {
            return Err("usage: midi-hook test [--details] [port-index]".into());
        }
        return test_midi(requested_port.as_deref(), details);
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
            "device = test\noutput = lights\n60 = shortcut 57:1 35:1 35:0 57:0\n61 = key 29\n62 = command echo hi\n63 = toggle sleep 60\n62+60+61 = command combo\n48>50>52 = command sequence\ncc 64 = key 42",
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
            config.actions[&MidiTrigger::Chord(vec![63])],
            Action::Toggle("sleep 60".into())
        );
        assert_eq!(
            config.actions[&MidiTrigger::Chord(vec![60, 61, 62])],
            Action::Command("combo".into())
        );
        assert_eq!(config.output.as_deref(), Some("lights"));
        assert_eq!(
            config.actions[&MidiTrigger::Sequence(vec![48, 50, 52])],
            Action::Command("sequence".into())
        );
        assert_eq!(
            config.actions[&MidiTrigger::Control(64)],
            Action::HeldKey(42)
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
        let notes = [48, 50, 52];
        let failure = sequence_failure(&notes);
        let mut position = 0;
        assert!(!advance_sequence(&notes, &failure, &mut position, 48));
        assert!(!advance_sequence(&notes, &failure, &mut position, 49));
        assert_eq!(position, 0);
        assert!(!advance_sequence(&notes, &failure, &mut position, 48));
        assert!(!advance_sequence(&notes, &failure, &mut position, 50));
        assert!(advance_sequence(&notes, &failure, &mut position, 52));
        let notes = [48, 48, 50];
        let failure = sequence_failure(&notes);
        let mut position = 0;
        for note in [48, 48, 48] {
            assert!(!advance_sequence(&notes, &failure, &mut position, note));
        }
        assert!(advance_sequence(&notes, &failure, &mut position, 50));
        let completed = vec![sequence_note_set(&[93, 94, 95])];
        assert!(sequence_takes_priority(&[93, 94, 95], &completed));
        assert!(!sequence_takes_priority(&[93, 94], &completed));
        assert_eq!(
            expand_cc_command("volume {value} {percent}", 64),
            Some("volume 64 50".into())
        );
        assert_eq!(
            expand_cc_command("volume {percent}", 127),
            Some("volume 100".into())
        );
        assert_eq!(expand_cc_command("volume 5%-", 0), None);
        assert_eq!(
            expand_cc_command("volume {percent}", 0),
            Some("volume 0".into())
        );

        let (sender, receiver) = mpsc::channel();
        for (note, pressed) in [(61, true), (60, true), (60, false), (61, false)] {
            sender.send(MidiInputEvent::Note(note, pressed)).unwrap();
        }
        assert_eq!(
            capture_midi_trigger(&receiver).unwrap(),
            MidiCapture::Notes(vec![61, 60])
        );
        let (sender, receiver) = mpsc::channel();
        sender.send(MidiInputEvent::Control(64, 127)).unwrap();
        assert_eq!(
            capture_midi_trigger(&receiver).unwrap(),
            MidiCapture::Control(64)
        );
        let updated = update_config(
            "device = old\n61+60 = command old\n",
            "new",
            Some("lights"),
            &MidiTrigger::Chord(vec![60, 61]),
            "command new",
        )
        .unwrap();
        let updated = parse_config(&updated).unwrap();
        assert_eq!(updated.device.as_deref(), Some("new"));
        assert_eq!(updated.output.as_deref(), Some("lights"));
        assert_eq!(
            updated.actions[&MidiTrigger::Chord(vec![60, 61])],
            Action::Command("new".into())
        );
        assert_eq!(midi_note(&[0x90, 60, 100]), Some((60, true)));
        assert_eq!(midi_note(&[0x90, 60, 0]), Some((60, false)));
        assert_eq!(midi_control(&[0xb0, 64, 127]), Some((64, 127)));
        assert_eq!(midi_control(&[0xb0, 64, 0]), Some((64, 0)));
        assert!(parse_config("60 = shortcut 57:1").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn toggles_command_process() {
        let trigger = MidiTrigger::Chord(vec![60]);
        let mut running = HashMap::new();
        toggle_command(&mut running, &trigger, "exec sleep 30");
        assert!(running.contains_key(&trigger));
        toggle_command(&mut running, &trigger, "exec sleep 30");
        assert!(!running.contains_key(&trigger));
    }
}
