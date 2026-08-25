# MIDI Controller

Run a shell command when a MIDI keyboard key is pressed.

```sh
cp commands.conf.example commands.conf
# Edit commands.conf, then:
cargo run --release -- commands.conf
```

The app lists MIDI inputs and asks which one to use. You can provide the input index directly:

```sh
cargo run --release -- commands.conf 0
```

Each non-comment config line maps a MIDI note number (`0`–`127`) to a command:

```text
60 = notify-send "Middle C pressed"
61 = playerctl play-pause
```

The app prints unmapped note numbers to help you configure the keyboard. It runs commands through `/bin/sh -c`, so only use a config file you trust. Restart the app after you edit the config. Press Enter to quit.
