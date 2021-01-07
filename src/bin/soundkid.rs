extern crate clap;

extern crate soundkid;

use soundkid::config;
use soundkid::reader;

use clap::{crate_version, App, Arg};
use config::Config;
use gpio_cdev::{Chip, EventRequestFlags, LineRequestFlags};
use log::info;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::env;
use std::process::{Child, Command};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

fn main() {
    env_logger::init();
    info!("Starting soundkid ...");

    let matches = App::new("soundkid")
       .version(crate_version!())
       .about("Sound player for kids")
        .author("Thomas Bechtold <thomasbechtold@jpberlin.de>")
        .arg(Arg::with_name("input_device_description")
             .long("input_device_description")
             .takes_value(true)
             .help("The input device description (usually a NFC reader) to use. Eg. '/dev/input/event0'"))
       .get_matches();

    // get the configuration
    let conf = Config::new();

    let mut input_device_desc = String::new();

    // command line argument wins against config file parameter
    if matches.occurrences_of("input_device_description") > 0 {
        input_device_desc = String::from(matches.value_of("input_device_description").unwrap());
    } else {
        input_device_desc = conf.common.input_device_description.clone();
    }

    // handle input
    let (reader_tx, reader_rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    thread::spawn(|| {
        let reader = reader::Input::new(input_device_desc);
        match reader {
            Some(r) => {
                r.handle_input(reader_tx);
            }
            _ => {
                panic!("Unable to create a input handler for the given input device description. Abort");
            }
        }
    });

    // the player process
    let mut child: Option<Child> = None;

    // handle GPIO input
    if !conf.gpio.is_empty() {
        for (gpio_device, gpio_device_actions) in conf.gpio.clone() {
            info!("Found config for GPIO device {}", gpio_device);
            for (gpio_line, gpio_action) in gpio_device_actions {
                let dev = gpio_device.clone();
                info!("Watching GPIO line {} on device {} now", gpio_line, dev);
                thread::spawn(move || {
                    let mut chip = Chip::new(dev).unwrap();
                    let input = chip.get_line(gpio_line).unwrap();
                    for _event in input
                        .events(
                            LineRequestFlags::INPUT,
                            // NOTE: do we need to handle also RISING_EDGE?
                            EventRequestFlags::FALLING_EDGE,
                            "mirror-gpio",
                        )
                        .unwrap()
                    {
                        // FIXME: DRY and PAUSE & RESUME are currently not supported
                        if gpio_action == "VOLUME_INCREASE" {
                            volume_increase();
                        } else if gpio_action == "VOLUME_DECREASE" {
                            volume_decrease();
                        }
                    }
                });
            }
        }
    } else {
        info!("No GPIO config found. Skipping GPIO handling");
    }

    for received in reader_rx {
        if conf.tags.contains_key(&received) {
            info!("Found key '{:?}' in tags", conf.tags[&received]);

            if conf.tags.get(&received).unwrap() == "PAUSE" {
                pause(&mut child);
            } else if conf.tags.get(&received).unwrap() == "RESUME" {
                resume(&mut child);
            } else if conf.tags.get(&received).unwrap() == "VOLUME_INCREASE" {
                volume_increase();
            } else if conf.tags.get(&received).unwrap() == "VOLUME_DECREASE" {
                volume_decrease();
            } else {
                play(&mut child, &conf, &received);
            }
        } else {
            info!("Received an unknown tag: {:?}", received);
        }
    }
}

/// pause the current child (soundkid-player) process
fn pause(child: &mut Option<Child>) {
    if let Some(x) = child {
        info!("Pause process with PID {:?}", x.id());
        signal::kill(Pid::from_raw(x.id() as i32), Signal::SIGTSTP).unwrap();
    }
}

/// resume the current child (soundkid-player) process
fn resume(child: &mut Option<Child>) {
    if let Some(x) = child {
        info!("Resume process with PID {:?}", x.id());
        signal::kill(Pid::from_raw(x.id() as i32), Signal::SIGCONT).unwrap();
    }
}

/// increase the volume via alsa
fn volume_increase() {
    // FIXME: do not hardcode the alsa mixer name
    let _output = Command::new("amixer")
        .args(&["set", "SoftMaster", "5%+"])
        .output()
        .expect("failed to increase volume via amixer");
}

/// increase the volume via alsa
fn volume_decrease() {
    // FIXME: do not hardcode the alsa mixer name
    let _output = Command::new("amixer")
        .args(&["set", "SoftMaster", "5%-"])
        .output()
        .expect("failed to decrease volume via amixer");
}

fn play(child: &mut Option<Child>, conf: &Config, tag: &String) {
    if let Some(x) = child {
        info!("Killing current player processs with PID: {}", x.id());
        x.kill().unwrap();
    }
    //start a new child
    // possible path to the soundkid-player binary (for debugging)
    let mut path = env::current_dir().unwrap();
    path.push("target");
    path.push("debug");

    info!(
        "Starting soundkid-player process for tag {} / uri {}...",
        tag,
        conf.tags.get(tag).unwrap()
    );
    *child = Some(
        Command::new("soundkid-player")
            // FIXME: do not hardcode the path
            .env("PATH", path.into_os_string().into_string().unwrap())
            .arg(conf.spotify.username.clone())
            .arg(conf.spotify.password.clone())
            .arg(conf.tags.get(tag).unwrap().clone())
            .spawn()
            .expect("Unable to spawn a child process"),
    );
}
