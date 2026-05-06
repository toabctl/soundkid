use anyhow::{Context, Result, anyhow};
use evdev::{Device, EventSummary, KeyCode};
use futures::stream::StreamExt;
use glob::glob;
use gpio_cdev::{AsyncLineEventHandle, Chip, EventRequestFlags, LineRequestFlags};
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, info, warn};

use crate::input::InputEvent;

pub struct Input {
    pub device_desc: String,
    pub device: Device,
}

impl Input {
    pub(crate) fn is_char_device(path: &str) -> bool {
        Path::new(path)
            .metadata()
            .map(|m| m.file_type().is_char_device())
            .unwrap_or(false)
    }

    /// Resolve `device_desc` to a `/dev/input/event*` path.
    /// Either it's already a path, or it's a device name that matches one.
    fn find_device_path(device_desc: &str) -> Option<String> {
        if Self::is_char_device(device_desc) {
            return Some(device_desc.to_string());
        }

        for entry in glob("/dev/input/event*").expect("invalid glob") {
            let path = match entry {
                Ok(p) => p,
                Err(e) => {
                    debug!("glob entry error: {e}");
                    continue;
                }
            };
            let path_str = path.to_string_lossy().into_owned();
            debug!("checking device path {path_str:?}");
            match Device::open(&path_str) {
                Ok(d) if d.name() == Some(device_desc) => return Some(path_str),
                Ok(_) => {}
                Err(e) => debug!("could not open {path_str:?}: {e}"),
            }
        }
        None
    }

    pub fn new(device_desc: &str) -> Result<Self> {
        let path = Self::find_device_path(device_desc).ok_or_else(|| {
            error!(
                "Cannot resolve device description {device_desc:?}. \
                 Try a path in /dev/input/ or a device name (e.g. from `evtest`)."
            );
            anyhow!("input device {device_desc:?} not found")
        })?;
        let device = Device::open(&path)?;
        info!("Using input device {path:?}");
        Ok(Self {
            device_desc: device_desc.to_string(),
            device,
        })
    }
}

/// Spawn a task that reads numeric scancodes from an evdev device, accumulates
/// them until ENTER, and pushes the resulting string into the channel as an
/// `InputEvent::Evdev`.
pub fn spawn_evdev_reader(reader: Input, tx: Sender<InputEvent>) {
    tokio::spawn(async move {
        let mut stream = match reader.device.into_event_stream() {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "could not turn {:?} into event stream: {e}",
                    reader.device_desc
                );
                return;
            }
        };
        let mut buf = String::new();
        loop {
            let ev = match stream.next_event().await {
                Ok(ev) => ev,
                Err(e) => {
                    warn!("evdev read error on {:?}: {e}", reader.device_desc);
                    return;
                }
            };
            // value 1 == key down; ignore key-up and key-repeat to avoid duplicates
            if ev.value() != 1 {
                continue;
            }
            let EventSummary::Key(_, code, _) = ev.destructure() else {
                continue;
            };
            match code {
                KeyCode::KEY_0 => buf.push('0'),
                KeyCode::KEY_1 => buf.push('1'),
                KeyCode::KEY_2 => buf.push('2'),
                KeyCode::KEY_3 => buf.push('3'),
                KeyCode::KEY_4 => buf.push('4'),
                KeyCode::KEY_5 => buf.push('5'),
                KeyCode::KEY_6 => buf.push('6'),
                KeyCode::KEY_7 => buf.push('7'),
                KeyCode::KEY_8 => buf.push('8'),
                KeyCode::KEY_9 => buf.push('9'),
                KeyCode::KEY_ENTER => {
                    if buf.is_empty() {
                        continue;
                    }
                    debug!("Input event on {:?}: {buf:?}", reader.device_desc);
                    let event = InputEvent::Evdev {
                        device: reader.device_desc.clone(),
                        scanned: buf.clone(),
                    };
                    if tx.send(event).await.is_err() {
                        return;
                    }
                    buf.clear();
                }
                _ => {}
            }
        }
    });
}

/// Open a GPIO line and return an async event handle ready to be consumed.
///
/// All the failure-prone setup (chip open, line lookup, event subscription,
/// AsyncFd registration) happens here so that callers can fail fast at startup
/// rather than discovering broken config inside a spawned task.
pub fn setup_gpio_line(chip_path: &str, line: u32) -> Result<AsyncLineEventHandle> {
    let mut chip =
        Chip::new(chip_path).with_context(|| format!("opening GPIO chip {chip_path:?}"))?;
    let chip_line = chip
        .get_line(line)
        .with_context(|| format!("getting GPIO line {line} on {chip_path:?}"))?;
    let events = chip_line
        .events(
            LineRequestFlags::INPUT,
            EventRequestFlags::FALLING_EDGE,
            "soundkid",
        )
        .with_context(|| format!("requesting events on GPIO {chip_path:?}/{line}"))?;
    AsyncLineEventHandle::new(events)
        .with_context(|| format!("registering async GPIO handle on {chip_path:?}/{line}"))
}

/// Spawn a task that consumes async GPIO events and forwards them as
/// `InputEvent::Gpio` into the channel.
pub fn spawn_gpio_reader(
    chip_path: String,
    line: u32,
    mut events: AsyncLineEventHandle,
    tx: Sender<InputEvent>,
) {
    tokio::spawn(async move {
        info!("Watching GPIO line {line} on {chip_path}");
        while let Some(event) = events.next().await {
            match event {
                Ok(ev) => debug!("GPIO event on {chip_path:?}/{line}: {ev:?}"),
                Err(e) => {
                    warn!("GPIO event error on {chip_path:?}/{line}: {e}");
                    return;
                }
            }
            let event = InputEvent::Gpio {
                chip: chip_path.clone(),
                line,
            };
            if tx.send(event).await.is_err() {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::Input;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn dev_null_is_a_char_device() {
        // /dev/null is universally a character device on Linux.
        assert!(Input::is_char_device("/dev/null"));
    }

    #[test]
    fn regular_file_is_not_a_char_device() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("regular");
        File::create(&path).unwrap();
        assert!(!Input::is_char_device(path.to_str().unwrap()));
    }

    #[test]
    fn nonexistent_path_is_not_a_char_device() {
        assert!(!Input::is_char_device("/this/does/not/exist/anywhere"));
    }

    #[test]
    fn empty_string_is_not_a_char_device() {
        assert!(!Input::is_char_device(""));
    }

    #[test]
    fn directory_is_not_a_char_device() {
        assert!(!Input::is_char_device("/tmp"));
    }
}
