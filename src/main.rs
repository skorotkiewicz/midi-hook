use midir::{Ignore, MidiInput};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::Command;
use std::thread;

fn parse_commands(text: &str) -> Result<HashMap<u8, String>, String> {
    let mut commands = HashMap::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (note, command) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: expected NOTE = COMMAND", index + 1))?;
        let note = note
            .trim()
            .parse::<u8>()
            .map_err(|_| format!("line {}: note must be between 0 and 127", index + 1))?;
        if note > 127 {
            return Err(format!(
                "line {}: note must be between 0 and 127",
                index + 1
            ));
        }
        let command = command.trim();
        if command.is_empty() {
            return Err(format!("line {}: command is empty", index + 1));
        }
        if commands.insert(note, command.to_owned()).is_some() {
            return Err(format!("line {}: note {note} is already mapped", index + 1));
        }
    }
    if commands.is_empty() {
        return Err("config contains no commands".into());
    }
    Ok(commands)
}

fn note_on(message: &[u8]) -> Option<u8> {
    (message.len() >= 3
        && message[0] & 0xf0 == 0x90
        && message[1] <= 127
        && (1..=127).contains(&message[2]))
    .then_some(message[1])
}

fn execute(command: String) {
    thread::spawn(
        move || match Command::new("/bin/sh").arg("-c").arg(&command).status() {
            Ok(status) if !status.success() => eprintln!("command failed ({status}): {command}"),
            Err(error) => eprintln!("could not run command: {error}"),
            _ => {}
        },
    );
}

fn choose_port(input: &MidiInput, requested: Option<&str>) -> Result<usize, String> {
    let ports = input.ports();
    if ports.is_empty() {
        return Err("no MIDI input ports found".into());
    }
    println!("MIDI inputs:");
    for (index, port) in ports.iter().enumerate() {
        let name = input
            .port_name(port)
            .unwrap_or_else(|_| "unknown device".into());
        println!("  {index}: {name}");
    }
    if let Some(index) = requested {
        return index
            .parse::<usize>()
            .ok()
            .filter(|index| *index < ports.len())
            .ok_or_else(|| format!("port index must be between 0 and {}", ports.len() - 1));
    }

    print!("Select input: ");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut selection = String::new();
    io::stdin()
        .read_line(&mut selection)
        .map_err(|error| error.to_string())?;
    selection
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|index| *index < ports.len())
        .ok_or_else(|| format!("port index must be between 0 and {}", ports.len() - 1))
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let config_path = args
        .next()
        .ok_or("usage: midi-controller <commands.conf> [port-index]")?;
    let requested_port = args.next();
    if args.next().is_some() {
        return Err("usage: midi-controller <commands.conf> [port-index]".into());
    }
    let commands = parse_commands(
        &fs::read_to_string(&config_path)
            .map_err(|error| format!("could not read {config_path}: {error}"))?,
    )?;

    let mut input = MidiInput::new("midi-controller").map_err(|error| error.to_string())?;
    input.ignore(Ignore::None);
    let port_index = choose_port(&input, requested_port.as_deref())?;
    let ports = input.ports();
    let port = ports
        .get(port_index)
        .ok_or_else(|| "MIDI input disappeared".to_string())?;
    let port_name = input
        .port_name(port)
        .unwrap_or_else(|_| "unknown device".into());
    let commands_for_input = commands;
    let _connection = input
        .connect(
            port,
            "midi-controller",
            move |_, message, _| {
                let Some(note) = note_on(message) else {
                    return;
                };
                if let Some(command) = commands_for_input.get(&note) {
                    println!("note {note}: {command}");
                    execute(command.clone());
                } else {
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
    Ok(())
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
    fn parses_mappings_and_only_accepts_positive_note_on() {
        let commands =
            parse_commands("# comment\n60 = echo middle=c\n61=playerctl play-pause\n").unwrap();
        assert_eq!(commands[&60], "echo middle=c");
        assert_eq!(note_on(&[0x90, 60, 100]), Some(60));
        assert_eq!(note_on(&[0x90, 60, 0]), None);
        assert_eq!(note_on(&[0x80, 60, 100]), None);
        assert!(parse_commands("128 = nope").is_err());
        assert!(parse_commands("60=a\n60=b").is_err());
    }
}
