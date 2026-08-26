![MIDI Hook banner](assets/banner.svg)

# MIDI Hook

Map MIDI notes to physical keyboard shortcuts, held keys, or shell commands on Linux, Windows, and macOS.

## Platform setup

### Linux

On Linux, setup captures keyboard input through `/dev/input`. Add your user to the `input` group, then log out and back in:

```sh
yay -S midi-hook # Arch Linux
sudo usermod -aG input "$USER"
```

Keyboard playback uses the kernel `uinput` interface. Make sure your user can write to `/dev/uinput`. MIDI Hook does not need a playback daemon.

### Windows

No extra keyboard software is required. MIDI Hook uses the native `SendInput` API.

### macOS

In System Settings → Privacy & Security, grant your terminal or MIDI Hook two permissions: Input Monitoring for capture and Accessibility for keyboard playback.

## Setup

```sh
cargo run --release -- setup
# midi-hook setup
```

Select a MIDI input, an optional MIDI output for LED feedback, and, on Linux, a physical keyboard. Move a MIDI control, or hold one or more MIDI notes and release them. For multiple notes, choose an unordered chord or an ordered sequence. Then choose an action:

```text
s  Press and release a physical shortcut
t  Type a shortcut such as ctrl+space+f4+c
c  Type a shell command
g  Type a toggle command
```

For a shortcut, hold all keys at the same time, then release them. Setup records the exact press and release sequence. MIDI Hook holds a single captured key, such as Ctrl, until you release the MIDI trigger. Press Esc to cancel capture if you selected the wrong keyboard.

Setup saves each mapping to `commands.conf`, then waits for another MIDI trigger. Press Ctrl+C while it waits to exit.

## Test MIDI input

Print each pressed MIDI note number without running any mappings:

```sh
cargo run --release -- test
# Optional MIDI port index:
# midi-hook test 0
```

Detailed diagnostics print every incoming MIDI message as hexadecimal bytes, including SysEx, clock, pitch bend, aftertouch, and unknown messages. Note messages also show the decoded held-note state and arrival order:

```sh
cargo run --release -- test --details
# Optional MIDI port index:
# midi-hook test --details 0
```

Press Enter to stop.

## Listen

```sh
cargo run --release -- commands.conf
# midi-hook commands.conf
```

The listener connects to the saved MIDI device. Press Enter to stop it.

### Start automatically on Linux

Packaged Linux releases include an optional systemd user service. Store the configuration in the expected location, then enable it:

```sh
mkdir -p ~/.config/midi-hook
cp commands.conf ~/.config/midi-hook/commands.conf
systemctl --user enable --now midi-hook
```

View logs or stop the service with:

```sh
journalctl --user -u midi-hook -f
systemctl --user stop midi-hook
```

The package installs the service but never enables it automatically. When stdin is not interactive, MIDI Hook waits for SIGINT instead of exiting on EOF.

## Configuration

```text
device = RockJam BT MIDI:RockJam BT MIDI Bluetooth 128:0
output = RockJam BT MIDI:RockJam BT MIDI Bluetooth 128:0
60 = shortcut 57:1 35:1 35:0 57:0
61 = shortcut ctrl+space+f4+c
62 = key 29
63 = command notify-send "MIDI note 63"
64 = toggle spotify
65 [vel=1..49] = command notify-send "Soft press"
65 [vel=50..99] = command notify-send "Middle press"
65 [vel=100..127] = command notify-send "Hard press"
60+61+62 = command notify-send "MIDI chord pressed"
48>50>52 = command notify-send "MIDI sequence entered"
cc 64 = key 29
cc 76 = command wpctl set-volume @DEFAULT_SINK@ {value}%
pitch = command echo "{value}"
```

A `+`-separated trigger runs once when you hold all listed MIDI notes. Releasing any required note re-arms it. A `>`-separated trigger requires note-on events in the listed order. A wrong note resets the sequence, and sequences have no timeout. Single-note mappings continue to run independently.

Single-note velocity conditions use inclusive ranges such as `65 [vel=50..99]`. Ranges must use values from 1 through 127. Chords and sequences ignore velocity. NoteOff releases held keys and LED state.

A `cc N` mapping activates held keys and shortcuts when control N has a value from 1 through 127, then releases them at 0. CC commands run for every nonzero value. Use `{value}` for the raw CC value from 0 to 127 or `{percent}` for a scaled value from 0 to 100. Commands with either placeholder also run at 0, which supports absolute knobs and faders.

A `pitch` command uses the same placeholders. Its raw range is 0 to 16383, and its scaled range is 0 to 100. MIDI Hook runs one CC or pitch command at a time per mapping. If another update arrives while the command is running, only the newest value remains queued.

When you configure `output`, active note mappings send MIDI NoteOn feedback. They send NoteOff when released. This can control controller LEDs. Sequences and CC mappings do not send LED feedback.

Manual keyboard shortcuts can use `+`-separated key names. Captured shortcuts and `key` actions store native key codes, so numeric mappings do not work across operating systems.

Commands run through `/bin/sh -c` on Linux and macOS, and `cmd.exe /C` on Windows. Commands must terminate or detach themselves because MIDI Hook does not impose a timeout.

A `toggle` action starts its command on the first trigger and stops its process tree on the next. MIDI Hook also stops active toggle commands when it exits. Launch the application executable directly. MIDI Hook does not kill an instance that was already running or a launcher that deliberately detaches.

Only use configuration files you trust.
