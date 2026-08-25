use midir::{Ignore, MidiInput};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

#[derive(Default)]
struct Config {
    device: Option<String>,
    commands: HashMap<u8, String>,
}

fn parse_config(text: &str) -> Result<Config, String> {
    let mut config = Config::default();
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: expected KEY = VALUE", index + 1))?;
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("line {}: value is empty", index + 1));
        }
        if key == "device" {
            if config.device.replace(value.to_owned()).is_some() {
                return Err(format!("line {}: device is already set", index + 1));
            }
            continue;
        }
        let note = key
            .parse::<u8>()
            .ok()
            .filter(|note| *note <= 127)
            .ok_or_else(|| format!("line {}: note must be between 0 and 127", index + 1))?;
        if config.commands.insert(note, value.to_owned()).is_some() {
            return Err(format!("line {}: note {note} is already mapped", index + 1));
        }
    }
    Ok(config)
}

fn update_config(text: &str, device: &str, note: u8, command: &str) -> Result<String, String> {
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
            lines.push(format!("{note} = {command}"));
            found_note = true;
        } else {
            lines.push(raw_line.to_owned());
        }
    }
    if !found_device {
        lines.push(format!("device = {device}"));
    }
    if !found_note {
        lines.push(format!("{note} = {command}"));
    }
    Ok(lines.join("\n") + "\n")
}

fn note_on(message: &[u8]) -> Option<u8> {
    (message.len() >= 3
        && message[0] & 0xf0 == 0x90
        && message[1] <= 127
        && (1..=127).contains(&message[2]))
    .then_some(message[1])
}

fn shell_command(command: &str) -> Command {
    let (program, flag) = if cfg!(windows) {
        ("cmd.exe", "/C")
    } else {
        ("/bin/sh", "-c")
    };
    let mut process = Command::new(program);
    process.arg(flag).arg(command);
    process
}

fn execute(command: String) {
    thread::spawn(move || match shell_command(&command).status() {
        Ok(status) if !status.success() => eprintln!("command failed ({status}): {command}"),
        Err(error) => eprintln!("could not run command: {error}"),
        _ => {}
    });
}

fn prompt(message: &str) -> Result<String, String> {
    print!("{message}");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut value = String::new();
    let bytes = io::stdin()
        .read_line(&mut value)
        .map_err(|error| error.to_string())?;
    if bytes == 0 {
        return Err("input closed".into());
    }
    Ok(value.trim().to_owned())
}

fn choose_port(
    input: &MidiInput,
    requested: Option<&str>,
    preferred: Option<&str>,
) -> Result<usize, String> {
    let ports = input.ports();
    if ports.is_empty() {
        return Err("no MIDI input ports found".into());
    }
    let names: Vec<String> = ports
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
        preferred.and_then(|preferred| names.iter().position(|candidate| candidate == preferred))
    {
        println!("Using MIDI input {index}: {}", names[index]);
        return Ok(index);
    }

    println!("MIDI inputs:");
    for (index, name) in names.iter().enumerate() {
        println!("  {index}: {name}");
    }
    loop {
        let selection = prompt("Select input: ")?;
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

fn setup(path: &Path) -> Result<Config, String> {
    let mut input = MidiInput::new("midi-hook-setup").map_err(|error| error.to_string())?;
    input.ignore(Ignore::None);
    let port_index = choose_port(&input, None, None)?;
    let ports = input.ports();
    let port = ports
        .get(port_index)
        .ok_or_else(|| "MIDI input disappeared".to_string())?;
    let device = input
        .port_name(port)
        .unwrap_or_else(|_| "unknown device".into());
    let (sender, receiver) = mpsc::sync_channel(1);
    println!("Press the MIDI key that should run the command...");
    let connection = input
        .connect(
            port,
            "midi-hook-setup",
            move |_, message, _| {
                if let Some(note) = note_on(message) {
                    let _ = sender.try_send(note);
                }
            },
            (),
        )
        .map_err(|error| error.to_string())?;
    let note = receiver
        .recv()
        .map_err(|_| "MIDI setup connection stopped".to_string())?;
    drop(connection);
    println!("Learned MIDI note {note}.");

    let command = loop {
        let command = prompt("Command: ")?;
        if !command.is_empty() {
            break command;
        }
        eprintln!("Command cannot be empty");
    };
    let existing = read_config(path)?;
    let updated = update_config(&existing, &device, note, &command)?;
    fs::write(path, &updated)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    println!("Saved {}. Starting listener...", path.display());
    parse_config(&updated)
}

fn listen(config: Config, requested_port: Option<&str>) -> Result<(), String> {
    if config.commands.is_empty() {
        return Err("config contains no commands; run `midi-hook setup`".into());
    }
    let mut input = MidiInput::new("midi-hook").map_err(|error| error.to_string())?;
    input.ignore(Ignore::None);
    let port_index = choose_port(&input, requested_port, config.device.as_deref())?;
    let ports = input.ports();
    let port = ports
        .get(port_index)
        .ok_or_else(|| "MIDI input disappeared".to_string())?;
    let port_name = input
        .port_name(port)
        .unwrap_or_else(|_| "unknown device".into());
    let commands = config.commands;
    let _connection = input
        .connect(
            port,
            "midi-hook",
            move |_, message, _| {
                let Some(note) = note_on(message) else {
                    return;
                };
                if let Some(command) = commands.get(&note) {
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

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let first = args.next().ok_or(
        "usage: midi-hook setup [commands.conf]\n       midi-hook <commands.conf> [port-index]",
    )?;
    if first == "setup" || first == "--setup" {
        let path = args.next().unwrap_or_else(|| "commands.conf".into());
        if args.next().is_some() {
            return Err("usage: midi-hook setup [commands.conf]".into());
        }
        return listen(setup(Path::new(&path))?, None);
    }

    let requested_port = args.next();
    if args.next().is_some() {
        return Err("usage: midi-hook <commands.conf> [port-index]".into());
    }
    let config = parse_config(
        &fs::read_to_string(&first).map_err(|error| format!("could not read {first}: {error}"))?,
    )?;
    listen(config, requested_port.as_deref())
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
    fn parses_and_updates_config_and_only_accepts_positive_note_on() {
        let existing = "# keep this\ndevice = old keyboard\n60 = old command\n";
        let updated = update_config(existing, "new keyboard", 60, "echo middle=c").unwrap();
        assert!(updated.contains("# keep this"));
        let config = parse_config(&updated).unwrap();
        assert_eq!(config.device.as_deref(), Some("new keyboard"));
        assert_eq!(config.commands[&60], "echo middle=c");
        assert_eq!(note_on(&[0x90, 60, 100]), Some(60));
        assert_eq!(note_on(&[0x90, 60, 0]), None);
        assert_eq!(note_on(&[0x80, 60, 100]), None);
        let shell = shell_command("echo test");
        let expected = if cfg!(windows) {
            ("cmd.exe", "/C")
        } else {
            ("/bin/sh", "-c")
        };
        assert_eq!(shell.get_program(), expected.0);
        assert_eq!(shell.get_args().next().unwrap(), expected.1);
        assert!(parse_config("128 = nope").is_err());
        assert!(parse_config("60=a\n60=b").is_err());
    }
}
