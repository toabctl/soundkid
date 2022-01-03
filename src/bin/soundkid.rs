extern crate clap;

extern crate soundkid;

use soundkid::config;
use soundkid::player;
use soundkid::reader;

use clap::{crate_version, App};
use config::Config;
use evdev::{InputEventKind, Key};
use futures::stream::StreamExt;
use gpio_cdev::{AsyncLineEventHandle, Chip, EventRequestFlags, LineRequestFlags};
use log::{debug, info, warn};
use std::env;
use tokio;
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
enum InputDeviceType {
    EVDEV,
    GPIO,
}

#[derive(Debug, Clone)]
struct InputChannelMessage {
    device_type: InputDeviceType,
    device: String,
    id: String,
}

fn handle_input_get_action(conf: &Config, message: &InputChannelMessage) -> Option<String> {
    match message.device_type {
        InputDeviceType::EVDEV => {
            if conf.input.contains_key(&message.device) {
                if conf.input[&message.device].contains_key(&message.id) {
                    let action = conf.input[&message.device][&message.id].clone();
                    return Some(action);
                } else {
                    info!(
			"Unable to handle message {:?} for input device {:?}. Message not configured",
			message.id, message.device
                    );
                }
            } else {
                info!(
                    "Unable to handle message {:?} for input device {:?}. Input device not in config",
                    message.id, message.device
		);
            }
        }
        InputDeviceType::GPIO => {
            info!("Got a GPIO message");
            if conf.gpio.contains_key(&message.device) {
                let message_id = &message.id.parse::<u32>().unwrap();
                if conf.gpio[&message.device].contains_key(message_id) {
                    let action = conf.gpio[&message.device][message_id].clone();
                    return Some(action);
                } else {
                    info!(
			"Unable to handle message {:?} for GPIO device {:?}. Message not configured",
			message.id, message.device
                    );
                }
            } else {
                info!(
                    "Unable to handle message {:?} for GPIO device {:?}. Input device not in config",
                    message.id, message.device
		);
            }
        }
    }
    None
}

/// increase the volume via alsa
async fn volume_increase(alsa_control: &str) {
    let _output = Command::new("amixer")
        .args(&["set", alsa_control, "5%+"])
        .output()
        .await
        .unwrap();
    info!("Increased volume for control {} by 5%", alsa_control);
}

/// decrease the volume via alsa
async fn volume_decrease(alsa_control: &str) {
    let _output = Command::new("amixer")
        .args(&["set", alsa_control, "5%-"])
        .output()
        .await
        .unwrap();
    info!("Decreased volume for control {} by 5%", alsa_control);
}

/// Handle incoming input events from the receiver channel
async fn handle_input(
    conf: &Config,
    mut events_rx: mpsc::Receiver<InputChannelMessage>,
    mut player: player::SpotifyPlayer,
) {
    info!("Input receiver started ...");
    while let Some(message) = events_rx.recv().await {
        debug!(
            "Received some message from {:?} {:?}: {:?}",
            message.device_type, message.device, message.id
        );
        let action = handle_input_get_action(&conf, &message);
        match action {
            Some(a) => {
                info!("Found received action '{:?}' in '{:?}'", a, message.device);
                match a.as_ref() {
                    "VOLUME_INCREASE" => {
                        volume_increase(&conf.alsa.control).await;
                    }
                    "VOLUME_DECREASE" => {
                        volume_decrease(&conf.alsa.control).await;
                    }
                    "PAUSE" => {
                        player.pause().await;
                    }
                    "RESUME" => {
                        player.resume().await;
                    }
                    _ => {
                        player.stop().await;
                        player.play(String::from(a)).await;
                    }
                }
            }
            None => warn!(
                "No action for message {:?} on device {:?}",
                message.id, message.device
            ),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    info!("Starting soundkid ...");

    let _matches = App::new("soundkid")
        .version(crate_version!())
        .about("Sound player for kids")
        .author("Thomas Bechtold <thomasbechtold@jpberlin.de>")
        .get_matches();

    // get the configuration
    let conf = Config::new();

    // channel to receive events
    let (events_tx, events_rx) = mpsc::channel(100);

    // list of readers for all input devices
    let mut readers = Vec::new();
    if !conf.input.is_empty() {
        for (input_device, _input_device_actions) in conf.input.clone() {
            let reader = reader::Input::new(&input_device);
            match reader {
                Some(r) => {
                    debug!("Got input reader {:?}", r.device_desc);
                    //let events = r.device.into_event_stream()?;
                    readers.push(r);
                    //event_handlers.push(events);
                }
                _ => {
                    panic!(
                        "Unable to create a reader for the given input device description. Abort"
                    );
                }
            }
        }
    } else {
        info!("No input config found. Skipping input handling");
    }

    for reader in readers {
        let thread_events_tx = events_tx.clone();
        tokio::task::spawn(async move {
            let mut event = reader.device.into_event_stream().unwrap();
            let mut input_str = String::new();
            loop {
                let ev = event.next_event().await.unwrap();
                // only handle value 1 - otherwise we have double events
                if ev.value() != 1 {
                    continue;
                }

                match ev.kind() {
                    InputEventKind::Key(Key::KEY_0) => input_str.push('0'),
                    InputEventKind::Key(Key::KEY_1) => input_str.push('1'),
                    InputEventKind::Key(Key::KEY_2) => input_str.push('2'),
                    InputEventKind::Key(Key::KEY_3) => input_str.push('3'),
                    InputEventKind::Key(Key::KEY_4) => input_str.push('4'),
                    InputEventKind::Key(Key::KEY_5) => input_str.push('5'),
                    InputEventKind::Key(Key::KEY_6) => input_str.push('6'),
                    InputEventKind::Key(Key::KEY_7) => input_str.push('7'),
                    InputEventKind::Key(Key::KEY_8) => input_str.push('8'),
                    InputEventKind::Key(Key::KEY_9) => input_str.push('9'),
                    InputEventKind::Key(Key::KEY_ENTER) => {
                        if input_str.len() > 0 {
                            let msg = InputChannelMessage {
                                device_type: InputDeviceType::EVDEV,
                                device: reader.device_desc.clone(),
                                id: input_str.clone(),
                            };
                            debug!(
                                "Input event on device {:?}: {:?}",
                                reader.device_desc, input_str
                            );
                            thread_events_tx.send(msg).await.unwrap();
                            input_str.clear();
                        }
                    }
                    _ => println!("No match"),
                };
            }
        });
    }

    // handle GPIO input
    if !conf.gpio.is_empty() {
        for (gpio_device, gpio_device_actions) in conf.gpio.clone() {
            info!("Found config for GPIO device {}", gpio_device);
            for (gpio_line, _) in gpio_device_actions {
                let dev = gpio_device.clone();
                let thread_events_tx = events_tx.clone();
                tokio::task::spawn(async move {
                    let mut chip = Chip::new(dev.clone()).unwrap();
                    let line = chip.get_line(gpio_line).unwrap();
                    let mut events = AsyncLineEventHandle::new(
                        line.events(
                            LineRequestFlags::INPUT,
                            // NOTE: do we need to handle also RISING_EDGE (BOTH_EDGES)?
                            EventRequestFlags::FALLING_EDGE,
                            "gpioevents",
                        )
                        .unwrap(),
                    )
                    .unwrap();
                    info!("Watching GPIO line {} on device {} now ...", gpio_line, dev);
                    loop {
                        match events.next().await {
                            Some(event) => {
                                let msg = InputChannelMessage {
                                    device_type: InputDeviceType::GPIO,
                                    device: dev.clone(),
                                    id: line.offset().to_string(),
                                };

                                thread_events_tx.send(msg).await.unwrap();
                                debug!(
                                    "GPIO event on device {:?} {:?}: {:?}",
                                    dev,
                                    line,
                                    event.unwrap()
                                );
                            }
                            None => break,
                        };
                    }
                });
            }
        }
    }

    // the spotify player
    let player =
        player::SpotifyPlayer::new(conf.spotify.username.clone(), conf.spotify.password.clone())
            .await;

    // play a startup sound
    //player.play(String::from("https://open.spotify.com/track/0PC1HwzqaRghfqVQxqelr8?si=0f8c23b4a0fe472a")).await;

    tokio::join!(handle_input(&conf, events_rx, player));

    Ok(())
}
