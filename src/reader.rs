use evdev::Device;
/// The reader module to get input events from the NFC card reader
use glob::glob;
use log::{debug, error, info};
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

pub struct Input {
    pub device_desc: String,
    pub device: Device,
}

impl Input {
    /// Check if the given device path seems to be ok
    fn check_device_path(device_path: &str) -> bool {
        let p = Path::new(device_path);
        if p.exists() {
            let meta = p.metadata().unwrap();
            let file_type = meta.file_type();
            if file_type.is_char_device() {
                return true;
            }
        }
        return false;
    }

    /// Try to find the given device
    fn find_device_path(device_desc: &str) -> Option<String> {
        // 1) just try the device string as path in the filesystem
        if Input::check_device_path(device_desc) == true {
            return Some(device_desc.to_string());
        }

        // 2) try to find the by device name (looping over all /dev/input files)
        for path in glob("/dev/input/event*").expect("Failed to read glob pattern for /dev/input") {
            let path_str = path.unwrap().into_os_string().into_string().unwrap();
            debug!("checking now device path {:?} ...", path_str);
            let d = evdev::Device::open(path_str.clone()).unwrap();
            // check the name against the given device description
            if d.name().unwrap() == device_desc {
                return Some(path_str);
            }
        }
        None
    }

    pub fn new(device_desc: &str) -> Option<Input> {
        // try first if the given string is a valid path
        let device_path = Input::find_device_path(device_desc);
        match device_path {
            Some(dp) => {
                let d = evdev::Device::open(dp.clone()).unwrap();
                info!("Using device {:?}", dp);
                let i = Input {
                    device_desc: device_desc.to_string(),
                    device: d,
                };
                return Some(i);
            }
            _ => {
                error!(
                    "Can not handle device description {:?}.
                           Try a path in /dev/input/ or a device name
                           (eg. list with 'evtest')",
                    device_desc
                );
            }
        }
        None
    }
}
