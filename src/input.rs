use crate::config::{Action, Config};

/// An event produced by one of the input readers (evdev keyboard scan, GPIO
/// line trigger).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Evdev { device: String, scanned: String },
    Gpio { chip: String, line: u32 },
}

/// Resolve an input event to the configured `Action`, or `None` if no mapping
/// exists for that event.
pub fn lookup_action<'a>(conf: &'a Config, ev: &InputEvent) -> Option<&'a Action> {
    match ev {
        InputEvent::Evdev { device, scanned } => {
            conf.input.get(device).and_then(|m| m.get(scanned))
        }
        InputEvent::Gpio { chip, line } => conf.gpio.get(chip).and_then(|m| m.get(line)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(yaml: &str) -> Config {
        serde_yaml_ng::from_str(yaml).expect("test config must parse")
    }

    const TRACK: &str = "6rqhFgbbKwnb9MLmUQDhG6";

    fn full_yaml() -> String {
        format!(
            r#"
alsa: {{ control: "Master" }}
spotify: {{}}
input:
  /dev/input/event0:
    "12345": "spotify:track:{TRACK}"
    "VOL_UP_CARD": "VOLUME_INCREASE"
gpio:
  /dev/gpiochip0:
    17: "PAUSE"
    27: "RESUME"
"#
        )
    }

    fn evdev(device: &str, scanned: &str) -> InputEvent {
        InputEvent::Evdev {
            device: device.into(),
            scanned: scanned.into(),
        }
    }

    fn gpio(chip: &str, line: u32) -> InputEvent {
        InputEvent::Gpio {
            chip: chip.into(),
            line,
        }
    }

    #[test]
    fn evdev_match_returns_action() {
        let c = config(&full_yaml());
        assert_eq!(
            lookup_action(&c, &evdev("/dev/input/event0", "12345")),
            Some(&Action::Play(format!("spotify:track:{TRACK}")))
        );
        assert_eq!(
            lookup_action(&c, &evdev("/dev/input/event0", "VOL_UP_CARD")),
            Some(&Action::VolumeIncrease)
        );
    }

    #[test]
    fn evdev_unknown_device_returns_none() {
        let c = config(&full_yaml());
        assert_eq!(
            lookup_action(&c, &evdev("/dev/input/event99", "12345")),
            None
        );
    }

    #[test]
    fn evdev_unknown_scanned_returns_none() {
        let c = config(&full_yaml());
        assert_eq!(
            lookup_action(&c, &evdev("/dev/input/event0", "99999")),
            None
        );
    }

    #[test]
    fn gpio_match_returns_action() {
        let c = config(&full_yaml());
        assert_eq!(
            lookup_action(&c, &gpio("/dev/gpiochip0", 17)),
            Some(&Action::Pause)
        );
        assert_eq!(
            lookup_action(&c, &gpio("/dev/gpiochip0", 27)),
            Some(&Action::Resume)
        );
    }

    #[test]
    fn gpio_unknown_line_returns_none() {
        let c = config(&full_yaml());
        assert_eq!(lookup_action(&c, &gpio("/dev/gpiochip0", 42)), None);
    }

    #[test]
    fn gpio_unknown_chip_returns_none() {
        let c = config(&full_yaml());
        assert_eq!(lookup_action(&c, &gpio("/dev/gpiochip9", 17)), None);
    }

    #[test]
    fn evdev_event_does_not_hit_gpio_mapping() {
        // gpio chip path used as evdev device must not cross-match.
        let c = config(&full_yaml());
        assert_eq!(lookup_action(&c, &evdev("/dev/gpiochip0", "17")), None);
    }

    #[test]
    fn gpio_event_does_not_hit_evdev_mapping() {
        let c = config(&full_yaml());
        assert_eq!(lookup_action(&c, &gpio("/dev/input/event0", 12345)), None);
    }

    #[test]
    fn empty_sections_return_none() {
        let c = config(
            r#"
alsa: {}
spotify: {}
"#,
        );
        assert_eq!(lookup_action(&c, &evdev("/dev/input/event0", "x")), None);
        assert_eq!(lookup_action(&c, &gpio("/dev/gpiochip0", 1)), None);
    }
}
