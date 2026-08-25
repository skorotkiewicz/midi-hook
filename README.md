![MIDI Hook banner](assets/banner.svg)

# MIDI Hook

Run a shell command when a MIDI keyboard key is pressed.

## Setup wizard

```sh
cargo run --release -- setup
```

The wizard:

1. Lists MIDI input devices.
2. Asks you to select one.
3. Learns the next key you press.
4. Asks for the command.
5. Saves `commands.conf` and starts listening.

Run the wizard again to add or replace another key mapping. Existing comments and other mappings are preserved.

## Start with saved settings

```sh
cargo run --release -- commands.conf
```

The saved device reconnects automatically. You can override it with a device index:

```sh
cargo run --release -- commands.conf 0
```

The config format is:

```text
device = RockJam BT MIDI:RockJam BT MIDI Bluetooth 128:0
60 = notify-send "Middle C pressed"
61 = playerctl play-pause
```

MIDI Hook does not let commands read its terminal input. Start interactive commands in a separate terminal. For example:

```text
61 = kitty pi -c
```

## Use MIDI keys as computer keys

On Linux with Wayland, `ydotool` can send normal keyboard input:

```text
60 = ydotool key 57:1 57:0           # Space
61 = ydotool key 28:1 28:0           # Enter
62 = ydotool key 29:1 46:1 46:0 29:0 # Ctrl+C
```

Start `ydotoold` and give it access to `/dev/uinput` before you start MIDI Hook.

The app prints unmapped note numbers to help with configuration. It uses `/bin/sh -c` on Linux and macOS. It uses `cmd.exe /C` on Windows. Only use a config file you trust.

Restart the listener after manual config edits. Press Enter to quit.
