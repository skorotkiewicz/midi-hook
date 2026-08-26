![MIDI Hook banner](assets/banner.svg)

# MIDI Hook

Map MIDI notes, chords, sequences, pedals, knobs, faders, and pitch bend to keyboard actions or shell commands on Linux, Windows, and macOS.

## Install

Arch Linux:

```sh
yay -S midi-hook
```

Other platforms can download a binary from [GitHub Releases](https://github.com/skorotkiewicz/midi-hook/releases) or build from source:

```sh
cargo build --release
```

Before setup, follow the short [platform permission instructions](REFERENCE.md#platform-setup).

## Quick start

Create mappings interactively:

```sh
midi-hook setup
```

Test what your controller sends:

```sh
midi-hook test
midi-hook test --details  # Every raw MIDI message
```

Start listening:

```sh
midi-hook commands.conf
```

Press Enter or Ctrl+C to stop.

## Configuration example

```text
device = RockJam BT MIDI:RockJam BT MIDI Bluetooth 128:0
60 = shortcut ctrl+alt+t
61 = command echo "note 61"
60+64+67 = command echo "chord"
48>50>52 = command echo "sequence"
65 [vel=100..127] = command echo "hard press"
cc 64 = command echo "pedal {value}"
cc 76 = command echo "volume {percent}%"
pitch = command echo "pitch {value}"
```

Run `midi-hook setup` instead of writing native key codes by hand. See [REFERENCE.md](REFERENCE.md) for every trigger, action, platform requirement, LED feedback, and command behavior.

## Start automatically on Linux

Packaged Linux releases include an optional systemd user service:

```sh
mkdir -p ~/.config/midi-hook
cp commands.conf ~/.config/midi-hook/commands.conf
systemctl --user enable --now midi-hook
```

View its logs:

```sh
journalctl --user -u midi-hook -f
```

The package installs the service but never enables it automatically.
