use super::{choose_midi_port, midi_note, note_numbers, sequence_note_set};
use midir::{Ignore, MidiInput};
use std::collections::HashSet;
use std::io::{self, Write};

#[derive(Default)]
struct MidiTestState {
    held: HashSet<u8>,
    order: Vec<u8>,
}

impl MidiTestState {
    fn update(&mut self, note: u8, pressed: bool) -> (bool, bool) {
        if pressed {
            let changed = self.held.insert(note);
            if changed {
                self.order.push(note);
            }
            (changed, false)
        } else {
            let changed = self.held.remove(&note);
            (changed, changed && self.held.is_empty())
        }
    }
}

pub(crate) fn test_midi(requested_port: Option<&str>, details: bool) -> Result<(), String> {
    let mut input = MidiInput::new("midi-hook-test").map_err(|error| error.to_string())?;
    input.ignore(Ignore::None);
    let port_index = choose_midi_port(&input, requested_port, None)?;
    let ports = input.ports();
    let port = ports.get(port_index).ok_or("MIDI input disappeared")?;
    let port_name = input
        .port_name(port)
        .unwrap_or_else(|_| "unknown device".into());
    let mut state = MidiTestState::default();
    let mut first_note = true;
    let connection = input
        .connect(
            port,
            "midi-hook-test",
            move |_, message, _| {
                let Some((note, pressed)) = midi_note(message) else {
                    return;
                };
                if !details {
                    if pressed {
                        if first_note {
                            print!("{note}");
                            first_note = false;
                        } else {
                            print!("+{note}");
                        }
                        state.held.insert(note);
                        let _ = io::stdout().flush();
                    } else {
                        state.held.remove(&note);
                        if state.held.is_empty() && !first_note {
                            println!();
                            first_note = true;
                        }
                    }
                    return;
                }
                let (changed, completed) = state.update(note, pressed);
                if !changed {
                    return;
                }
                let held = sequence_note_set(&state.held.iter().copied().collect::<Vec<_>>());
                let held = if held.is_empty() {
                    "-".into()
                } else {
                    note_numbers(&held, "+")
                };
                if pressed {
                    println!(
                        "down {note}  held: {held}  order: {}",
                        note_numbers(&state.order, ">")
                    );
                } else {
                    println!("up   {note}  held: {held}");
                }
                if completed {
                    println!(
                        "chord: {}\norder: {}\n",
                        note_numbers(&sequence_note_set(&state.order), "+"),
                        note_numbers(&state.order, ">")
                    );
                    state.order.clear();
                }
            },
            (),
        )
        .map_err(|error| error.to_string())?;
    println!("Testing {port_name}. Press MIDI notes; press Enter to quit.");
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    drop(connection);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_held_notes_and_order() {
        let mut state = MidiTestState::default();
        assert_eq!(state.update(50, true), (true, false));
        assert_eq!(state.update(48, true), (true, false));
        assert_eq!(state.update(50, false), (true, false));
        assert_eq!(state.update(48, false), (true, true));
        assert!(state.held.is_empty());
        assert_eq!(state.order, vec![50, 48]);
    }
}
