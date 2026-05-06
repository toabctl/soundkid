use anyhow::{Result, anyhow};
use evdev::Device;
use glob::glob;
use log::{debug, error, info};
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

pub struct Input {
    pub device_desc: String,
    pub device: Device,
}

impl Input {
    fn is_char_device(path: &str) -> bool {
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
