![MIDI Hook banner](assets/banner.svg)

# MIDI Hook

Map MIDI notes to physical keyboard shortcuts, held keys, or shell commands on Linux.

## Permissions

Setup reads the keyboard from `/dev/input`. Add your user to the `input` group, then log out and back in:

```sh
sudo usermod -aG input "$USER"
```

Keyboard playback requires `ydotoold`.

## Setup

```sh
cargo run --release -- setup
```

Select a MIDI input and physical keyboard. Hold one or more MIDI notes, release them all, then choose:

```text
s  Press and release a physical shortcut
t  Type a shortcut such as ctrl+space+f4+c
c  Type a shell command
```

Hold all keys in a shortcut at the same time, then release them. Setup records the exact press/release sequence. A single captured key, such as Ctrl, is held until the MIDI trigger is released. Press Esc to cancel physical capture if you selected the wrong keyboard.

Setup saves `commands.conf` and waits for the next MIDI trigger. Press Ctrl+C while it waits to exit.

## Listen

```sh
cargo run --release -- commands.conf
```

The listener reconnects to the saved MIDI device. Press Enter to stop it.

## Configuration

```text
device = RockJam BT MIDI:RockJam BT MIDI Bluetooth 128:0
60 = shortcut 57:1 35:1 35:0 57:0
61 = shortcut ctrl+space+f4+c
62 = key 29
63 = command notify-send "MIDI note 63"
60+61+62 = command notify-send "MIDI chord pressed"
```

A `+`-separated MIDI trigger runs once when all listed MIDI notes are held. It re-arms when any required note is released. Single-note mappings still run independently.

Manual keyboard shortcuts can use `+`-separated key names. Captured shortcuts store exact Linux input event codes and values. `key` stores one Linux input code. `command` runs through `/bin/sh -c`. Only use configuration files you trust.
