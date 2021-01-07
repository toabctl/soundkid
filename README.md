# soundkid - a spotify music player for kids

soundkid is software that can be used by kids for playing music from
[Spotify](https://www.spotify.com/) without using a complicated interface
like a display. Instead it can be used with a RFID Reader 
(I'm currently using a [Neuftech USB RFID Reader](https://www.amazon.de/Neuftech-Reader-Kartenleseger%C3%A4t-Kartenleser-Kontaktlos/dp/B018OYOR3E)) to interact with the music box.

soundkid is inspired by [Phoniebox](http://phoniebox.de/) 
and [Toniebox](https://tonies.de/) and uses 
[librespot](https://github.com/librespot-org/librespot) to interact with Spotify.
I started my own project instead of using Phoniebox to learn more about the
[Rust language](https://www.rust-lang.org/).

## Overview

soundkid has 2 executables - `soundkid-player` and `soundkid`.

`soundkid-player` can be used from the command line to just play something from spotify:

```
soundkid-player <spotify_username> <spotify_password> <spotify_id>
```

Using `soundkid-player` directly is mostly useful for debugging. Usually, `soundkid`
handles everything.

`soundkid` itself does read a configuration file from `~/.soundkid.conf` and waits for
input events from the input device (eg. the RFID Reader) and then starts
`soundkid-player` to play something.

## Configuration
The configuration must be located at `~/.soundkid.conf` and should be YAML. Here's an example:
```
---
gpio:  # GPIO config is optional
  "/dev/gpiochip0":  # this is the GPIO device path
    5: "VOLUME_DECREASE"  # this is a mapping between the GPIO pin (5 in this case) and the action
    6: "VOLUME_INCREASE"  # another mapping
input:
  "HXGCoLtd Keyboard":
    00000044886655661122: "spotify:playlist:43nVldajDhG1YVwZKxVh"
    00000011559977882233: "https://open.spotify.com/album/7LQhG0xSDjFiKJnziyB3Zj?si=eFQbWbq0Q16q6Go8tjlCvw"
    00000044772255668800: "https://open.spotify.com/album/5N73vwGXol4maS9U6HLp0o?si=oWBQKSpWReuOPnr50ayWSw"
    00000044772255668800: "PAUSE"
    00000011666611330099: "RESUME"
    00000011666611330100: "VOLUME_INCREASE"
    00000011666611330101: "VOLUME_DECREASE"
spotify:
  username: "my-spotify-username"
  password: "my-secret-spotify-password"
```

The `gpio` map is optional and can be used to map actions to GPIO pins. The `gpio` map contains another map
with the GPIO device name as key. The value is another key/value pair where the key (in the example 5 or 6)
is the GPIO pin and the value (in the example "VOLUME_DECREASE" and "VOLUME_INCREASE") is the action.

The `input` map is similar to the `gpio` map. "HXGCoLtd Keyboard" is the key (this could also be
eg. `/dev/input/event15`, try `sudo evtest` to get information about the available devices) and under 
that key is another map that contains key/value pairs of tags (in this example the values starting with 00000)
and actions (in this example spotify uris or other actions like "PAUSE", "RESUME", ...)

The `spotify` map must contain `username` and `password`.

The following actions (beside the spotify URIs) are currently available:

- `PAUSE`: pause the current `soundkid-player` process
- `RESUME` which resume the paused `soundkid-player` process
- `VOLUME_INCREASE` increase the volume by 5%
- `VOLUME_DECREASE` decrease the volume by 5%

## Debugging
Build the project with:

```
cargo build
```

Running `soundkid-player` directly can be done with:

```
RUST_BACKTRACE=full RUST_LOG=debug cargo run --bin soundkid-player -- -
```

Running `soundkid`:

```
RUST_BACKTRACE=full RUST_LOG=debug cargo run --bin soundkid
```

## Contributions

Please use [github pull requests](https://github.com/toabctl/soundkid/pulls) for code/doc changes
and [github issues](https://github.com/toabctl/soundkid/issues) to report problems or ask questions.

## License
The code is licensed under the [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) license.
