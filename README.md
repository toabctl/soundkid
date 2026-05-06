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

`soundkid` reads a YAML configuration file, watches the configured input
devices (RFID/keyboard via evdev, GPIO buttons via gpio-cdev), and dispatches
each event to either Spotify playback (via librespot) or ALSA volume control
(via `amixer`). Spotify playback runs in-process — there is no separate
`soundkid-player` binary anymore.

## Authentication

Spotify removed username/password authentication. soundkid now uses OAuth +
a credentials cache:

1. The first time you run soundkid it will print an authorization URL and
   (best-effort) open it in a browser. Sign in to Spotify; the app gets an
   access token.
2. The reusable credentials returned by the access point are persisted to the
   configured `cache_dir` so subsequent starts connect headlessly without a
   browser.

Run the first OAuth login interactively (e.g. `cargo run --bin soundkid` on
your desktop, or `systemctl stop soundkid && sudo -u soundkid soundkid` on the
target) before relying on the systemd service.

Spotify Premium is required.

## Configuration

soundkid looks for `~/.soundkid.conf` first, then `/etc/soundkid.conf`.
The first one that reads and parses cleanly wins.

```yaml
---
gpio:  # optional
  "/dev/gpiochip0":
    5: "VOLUME_DECREASE"
    6: "VOLUME_INCREASE"
input:  # optional
  "HXGCoLtd Keyboard":
    "00000044886655661122": "spotify:playlist:43nVldajDhG1YVwZKxVh"
    "00000011559977882233": "https://open.spotify.com/album/7LQhG0xSDjFiKJnziyB3Zj?si=eFQbWbq0Q16q6Go8tjlCvw"
    "00000044772255668801": "PAUSE"
    "00000011666611330099": "RESUME"
    "00000011666611330100": "VOLUME_INCREASE"
    "00000011666611330101": "VOLUME_DECREASE"
alsa:
  control: "SoftMaster"     # optional, default "Master"
spotify:
  cache_dir: "/var/lib/soundkid"   # optional, default ~/.cache/soundkid
  client_id: "..."                 # optional, default librespot's keymaster id
```

`gpio` maps a GPIO chip path → line offset → action.

`input` maps a device name (or `/dev/input/event*` path; `sudo evtest` lists
available devices) → scanned-id-string → action.

`alsa.control` is the mixer control name used by `amixer set <control>
5%+/5%-` (try `amixer` to list available controls).

`spotify.cache_dir` is where reusable credentials are stored after the first
OAuth login.

### Actions

Action values are validated at config load — typos are rejected at startup
rather than at first card scan.

- `VOLUME_INCREASE` — `amixer set <alsa.control> 5%+`
- `VOLUME_DECREASE` — `amixer set <alsa.control> 5%-`
- `PAUSE` — pause Spotify playback
- `RESUME` — resume Spotify playback
- A Spotify URI (`spotify:track:...`, `spotify:album:...`, `spotify:playlist:...`)
- An `https://open.spotify.com/...` URL (query strings like `?si=...` are stripped)

## Building

You'll need ALSA development headers:

```
# Debian/Ubuntu
sudo apt-get install build-essential libasound2-dev
# openSUSE
sudo zypper install alsa-devel
# Fedora
sudo dnf install alsa-lib-devel make gcc
```

Then:

```
cargo build --release
```

## Debugging

```
RUST_BACKTRACE=full RUST_LOG=debug cargo run --bin soundkid
```

## Building a .deb package

```
cargo install cargo-deb
cargo deb
```

## Contributions

Please use [github pull requests](https://github.com/toabctl/soundkid/pulls)
for code/doc changes and [github issues](https://github.com/toabctl/soundkid/issues)
to report problems or ask questions.

## License

The code is licensed under the [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) license.
