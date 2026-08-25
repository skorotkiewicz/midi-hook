#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::*;
#[cfg(target_os = "macos")]
pub(crate) use macos::*;
#[cfg(target_os = "windows")]
pub(crate) use windows::*;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("midi-hook supports Linux, Windows, and macOS");

pub(crate) fn terminal_escape_pressed() -> Result<bool, String> {
    let mut escape = false;
    while crossterm::event::poll(std::time::Duration::ZERO).map_err(|error| error.to_string())? {
        if matches!(
            crossterm::event::read().map_err(|error| error.to_string())?,
            crossterm::event::Event::Key(key)
                if key.kind == crossterm::event::KeyEventKind::Press
                    && key.code == crossterm::event::KeyCode::Esc
        ) {
            escape = true;
        }
    }
    Ok(escape)
}

pub(crate) struct RawMode;

impl RawMode {
    pub(crate) fn enable() -> Result<Self, String> {
        crossterm::terminal::enable_raw_mode()
            .map_err(|error| format!("could not enable raw terminal mode: {error}"))?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[derive(Default)]
pub(crate) struct Capture {
    active: std::collections::HashSet<u16>,
    pub(crate) events: Vec<(u16, i32)>,
    started: bool,
}

impl Capture {
    pub(crate) fn push(&mut self, code: u16, value: i32) -> bool {
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

    pub(crate) fn action(&self) -> String {
        let pressed: std::collections::HashSet<_> = self
            .events
            .iter()
            .filter_map(|(code, value)| (*value == 1).then_some(*code))
            .collect();
        if pressed.len() == 1 {
            return format!("key {}", pressed.into_iter().next().unwrap());
        }
        let events = self
            .events
            .iter()
            .map(|(code, value)| format!("{code}:{value}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("shortcut {events}")
    }
}
