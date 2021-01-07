/// The reader module to get input events from the NFC card reader
use evdev_rs::enums::EventCode;
use evdev_rs::enums::EventType;
use evdev_rs::enums::EV_KEY;
use evdev_rs::Device;
use glob::glob;
use log::{debug, error, info};
use std::fs::File;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

pub struct Input {
    device_desc: String,
    device: Device,
    input_str: String,
}

impl Input {
    /// Check if the given device path seems to be ok
    fn check_device_path(device_path: &String) -> bool {
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
    fn find_device_path(device_desc: &String) -> Option<String> {
        // 1) just try the device string as path in the filesystem
        if Input::check_device_path(device_desc) == true {
            return Some(device_desc.clone());
        }

        // 2) try to find the by device name (looping over all /dev/input files)
        for path in glob("/dev/input/event*").expect("Failed to read glob pattern for /dev/input") {
            let path_str = path.unwrap().into_os_string().into_string().unwrap();
            debug!("checking now device path {:?} ...", path_str);
            let f = File::open(path_str.clone());
            let f = match f {
                Ok(file) => file,
                Err(error) => {
                    error!("Problem opening the file: {:?}", error);
                    continue;
                }
            };
            let d = Device::new_from_fd(f).unwrap();
            // check the name against the given device description
            if d.name().unwrap() == device_desc {
                return Some(path_str.clone());
            }
        }
        None
    }

    pub fn new(device_desc: String) -> Option<Input> {
        // try first if the given string is a valid path
        let device_path = Input::find_device_path(&device_desc);
        match device_path {
            Some(dp) => {
                let f = File::open(dp.clone()).unwrap();
                let d = Device::new_from_fd(f).unwrap();
                info!("Using device {:?} ({:?})", dp, d.name().unwrap());
                let i = Input {
                    device_desc: device_desc,
                    device: d,
                    input_str: String::new(),
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

    /// handle the input from the input source (usually a NFC reader)
    ///
    /// * `input_device` - path to the input device. eg. /dev/input/event22
    pub fn handle_input(mut self, tx: std::sync::mpsc::Sender<String>) {
        loop {
            let a = self
                .device
                .next_event(evdev_rs::ReadFlag::NORMAL | evdev_rs::ReadFlag::BLOCKING);
            match a {
                Ok(k) => {
                    match k.1.event_type {
                        EventType::EV_KEY => {
                            match k.1.event_code {
                                EventCode::EV_KEY(EV_KEY::KEY_0) => self.input_str.push('0'),
                                EventCode::EV_KEY(EV_KEY::KEY_1) => self.input_str.push('1'),
                                EventCode::EV_KEY(EV_KEY::KEY_2) => self.input_str.push('2'),
                                EventCode::EV_KEY(EV_KEY::KEY_3) => self.input_str.push('3'),
                                EventCode::EV_KEY(EV_KEY::KEY_4) => self.input_str.push('4'),
                                EventCode::EV_KEY(EV_KEY::KEY_5) => self.input_str.push('5'),
                                EventCode::EV_KEY(EV_KEY::KEY_6) => self.input_str.push('6'),
                                EventCode::EV_KEY(EV_KEY::KEY_7) => self.input_str.push('7'),
                                EventCode::EV_KEY(EV_KEY::KEY_8) => self.input_str.push('8'),
                                EventCode::EV_KEY(EV_KEY::KEY_9) => self.input_str.push('9'),
                                EventCode::EV_KEY(EV_KEY::KEY_ENTER) => {
                                    if self.input_str.len() > 0 {
                                        //info!("GOT enter. process/send current command: {}", self.input_str);
                                        tx.send(self.input_str.clone()).unwrap();
                                        // cleanup current command
                                        self.input_str.clear();
                                    }
                                }
                                _ => (),
                            }
                        }
                        _ => (),
                    }
                }
                Err(_e) => (),
            }
        }
    }
}
