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
