extern crate clap;

extern crate soundkid;

use soundkid::config;
use soundkid::reader;

use clap::{crate_version, App};
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
use std::time::Instant;

fn main() {
    env_logger::init();
    info!("Starting soundkid ...");

    let _matches = App::new("soundkid")
        .version(crate_version!())
        .about("Sound player for kids")
        .author("Thomas Bechtold <thomasbechtold@jpberlin.de>")
        .get_matches();

    // get the configuration
    let conf = Config::new();

    // handle evdev input
    let (input_reader_tx, input_reader_rx): (Sender<(String, String)>, Receiver<(String, String)>) =
        mpsc::channel();
    if !conf.input.is_empty() {
        for (input_device, _input_device_actions) in conf.input.clone() {
            info!("Found config for input device {}", input_device);
            let ir_tx = input_reader_tx.clone();
            thread::spawn(|| {
                let reader = reader::Input::new(input_device);
                match reader {
                    Some(r) => {
                        r.handle_input(ir_tx);
                    }
                    _ => {
                        panic!("Unable to create a input handler for the given input device description. Abort");
                    }
                }
            });
        }
    } else {
        info!("No input config found. Skipping input handling");
    }

    // the player process
    let mut child: Option<Child> = None;

    // handle GPIO input
    if !conf.gpio.is_empty() {
        for (gpio_device, gpio_device_actions) in conf.gpio.clone() {
            info!("Found config for GPIO device {}", gpio_device);
            for (gpio_line, gpio_action) in gpio_device_actions {
                let dev = gpio_device.clone();
                let alsa_control = conf.alsa.control.clone();
                info!("Watching GPIO line {} on device {} now", gpio_line, dev);
                thread::spawn(move || {
                    let mut chip = Chip::new(dev.clone()).unwrap();
                    let input = chip.get_line(gpio_line).unwrap();
                    let mut last_falling_event: Instant = Instant::now();
                    for _event in input
                        .events(
                            LineRequestFlags::INPUT,
                            // NOTE: do we need to handle also RISING_EDGE?
                            EventRequestFlags::FALLING_EDGE,
                            "mirror-gpio",
                        )
                        .unwrap()
                    {
                        // very simple debounce method to not retrigger actions for bouncing buttons
                        if last_falling_event.elapsed().as_millis() > 200 {
                            // FIXME: DRY and PAUSE & RESUME are currently not supported
                            if gpio_action == "VOLUME_INCREASE" {
                                volume_increase(alsa_control.clone());
                            } else if gpio_action == "VOLUME_DECREASE" {
                                volume_decrease(alsa_control.clone());
                            }
                        } else {
                            info!("Not doing anything for GPIO falling event on dev {} line {} - 200 ms not passed since last falling event", dev, gpio_line);
                        }
                        last_falling_event = Instant::now();
                    }
                });
            }
        }
    } else {
        info!("No GPIO config found. Skipping GPIO handling");
    }

    for (input_device_desc, action) in input_reader_rx {
        if conf.input.contains_key(&input_device_desc) {
            if conf.input[&input_device_desc].contains_key(&action) {
                let action = conf.input[&input_device_desc][&action].clone();
                info!(
                    "Found received action '{:?}' in '{:?}'",
                    action, input_device_desc
                );

                if action == "PAUSE" {
                    pause(&mut child);
                } else if action == "RESUME" {
                    resume(&mut child);
                } else if action == "VOLUME_INCREASE" {
                    volume_increase(conf.alsa.control.clone());
                } else if action == "VOLUME_DECREASE" {
                    volume_decrease(conf.alsa.control.clone());
                } else {
                    play(&mut child, &conf, &action);
                }
            } else {
                info!(
                    "Received an unknown action '{:?}' for input device {:?}",
                    action, input_device_desc
                );
            }
        } else {
            info!("Received an unknown input device {:?}", input_device_desc)
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
fn volume_increase(alsa_control: String) {
    let _output = Command::new("amixer")
        .args(&["set", alsa_control.as_str(), "5%+"])
        .output()
        .expect("failed to increase volume via amixer");
    info!("Increased volume for control {} by 5%", alsa_control);
}

/// increase the volume via alsa
fn volume_decrease(alsa_control: String) {
    let _output = Command::new("amixer")
        .args(&["set", alsa_control.as_str(), "5%-"])
        .output()
        .expect("failed to decrease volume via amixer");
    info!("Decreased volume for control {} by 5%", alsa_control);
}

fn play(child: &mut Option<Child>, conf: &Config, action: &String) {
    if let Some(x) = child {
        info!("Killing current player processs with PID: {}", x.id());
        x.kill().unwrap();
    }
    //start a new child
    // possible path to the soundkid-player binary (for debugging)
    let mut path = env::current_dir().unwrap();
    path.push("target");
    path.push("debug");

    info!("Starting soundkid-player process for action {} ...", action);
    *child = Some(
        Command::new("soundkid-player")
            // FIXME: do not hardcode the path
            .env("PATH", path.into_os_string().into_string().unwrap())
            .arg(conf.spotify.username.clone())
            .arg(conf.spotify.password.clone())
            .arg(action.clone())
            .spawn()
            .expect("Unable to spawn a child process"),
    );
}
