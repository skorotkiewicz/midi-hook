use midir::{MidiOutput, MidiOutputConnection};
use std::collections::HashMap;

pub(super) struct MidiFeedback {
    connection: MidiOutputConnection,
    active: HashMap<u8, usize>,
}

impl MidiFeedback {
    pub(super) fn set_notes(&mut self, notes: &[u8], pressed: bool) {
        for note in notes {
            if pressed {
                let count = self.active.entry(*note).or_default();
                *count += 1;
                if *count == 1
                    && let Err(error) = self.connection.send(&[0x90, *note, 127])
                {
                    eprintln!("could not send MIDI feedback: {error}");
                }
            } else if let Some(count) = self.active.get_mut(note) {
                *count -= 1;
                if *count == 0 {
                    self.active.remove(note);
                    if let Err(error) = self.connection.send(&[0x80, *note, 0]) {
                        eprintln!("could not send MIDI feedback: {error}");
                    }
                }
            }
        }
    }

    pub(super) fn release_all(&mut self) {
        for note in self.active.drain().map(|(note, _)| note) {
            let _ = self.connection.send(&[0x80, note, 0]);
        }
    }
}

pub(super) fn connect_midi_feedback(name: Option<&str>) -> Result<Option<MidiFeedback>, String> {
    let Some(name) = name else {
        return Ok(None);
    };
    let output = MidiOutput::new("midi-hook-feedback").map_err(|error| error.to_string())?;
    let ports = output.ports();
    let port = ports
        .iter()
        .find(|port| output.port_name(port).ok().as_deref() == Some(name))
        .ok_or_else(|| format!("saved MIDI output not found: {name}"))?;
    let connection = output
        .connect(port, "midi-hook-feedback")
        .map_err(|error| error.to_string())?;
    println!("Using MIDI output: {name}");
    Ok(Some(MidiFeedback {
        connection,
        active: HashMap::new(),
    }))
}
