use anyhow::{Context, Result};
use clap::Parser;
use evdev::{EventSummary, KeyCode};
use futures::stream::StreamExt;
use gpio_cdev::{AsyncLineEventHandle, Chip, EventRequestFlags, LineRequestFlags};
use log::{debug, info, warn};
use soundkid::{
    config::Config,
    player::SpotifyPlayer,
    reader::Input,
};
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(name = "soundkid", version, about = "Sound player for kids")]
struct Cli {}

#[derive(Debug, Clone)]
enum InputDeviceType {
    Evdev,
    Gpio,
}

#[derive(Debug, Clone)]
struct InputMessage {
    device_type: InputDeviceType,
    device: String,
    id: String,
}

fn lookup_action<'a>(conf: &'a Config, msg: &InputMessage) -> Option<&'a str> {
    match msg.device_type {
        InputDeviceType::Evdev => conf
            .input
            .get(&msg.device)
            .and_then(|m| m.get(&msg.id))
            .map(String::as_str),
        InputDeviceType::Gpio => {
            let line: u32 = msg.id.parse().ok()?;
            conf.gpio
                .get(&msg.device)
                .and_then(|m| m.get(&line))
                .map(String::as_str)
        }
    }
}

async fn amixer(control: &str, change: &str) {
    match Command::new("amixer")
        .args(["set", control, change])
        .output()
        .await
    {
        Ok(_) => info!("Adjusted volume for {control} by {change}"),
        Err(e) => warn!("amixer set {control} {change} failed: {e}"),
    }
}

async fn handle_input(
    conf: Config,
    mut events_rx: mpsc::Receiver<InputMessage>,
    player: SpotifyPlayer,
) {
    info!("Input receiver started");
    while let Some(msg) = events_rx.recv().await {
        debug!(
            "Received message from {:?} {:?}: {:?}",
            msg.device_type, msg.device, msg.id
        );
        let Some(action) = lookup_action(&conf, &msg) else {
            warn!(
                "no action for message {:?} on device {:?}",
                msg.id, msg.device
            );
            continue;
        };
        info!("Action {action:?} on device {:?}", msg.device);
        match action {
            "VOLUME_INCREASE" => amixer(&conf.alsa.control, "5%+").await,
            "VOLUME_DECREASE" => amixer(&conf.alsa.control, "5%-").await,
            "PAUSE" => player.pause(),
            "RESUME" => player.resume(),
            other => player.play(other.to_string()),
        }
    }
}

fn spawn_evdev_reader(reader: Input, tx: mpsc::Sender<InputMessage>) {
    tokio::spawn(async move {
        let mut stream = match reader.device.into_event_stream() {
            Ok(s) => s,
            Err(e) => {
                warn!("could not turn {:?} into event stream: {e}", reader.device_desc);
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
                    let msg = InputMessage {
                        device_type: InputDeviceType::Evdev,
                        device: reader.device_desc.clone(),
                        id: buf.clone(),
                    };
                    if tx.send(msg).await.is_err() {
                        return;
                    }
                    buf.clear();
                }
                _ => {}
            }
        }
    });
}

fn spawn_gpio_reader(device: String, line_offset: u32, tx: mpsc::Sender<InputMessage>) {
    tokio::spawn(async move {
        let mut chip = match Chip::new(&device) {
            Ok(c) => c,
            Err(e) => {
                warn!("could not open GPIO chip {device:?}: {e}");
                return;
            }
        };
        let line = match chip.get_line(line_offset) {
            Ok(l) => l,
            Err(e) => {
                warn!("could not get GPIO line {line_offset} on {device:?}: {e}");
                return;
            }
        };
        let events = match line.events(
            LineRequestFlags::INPUT,
            EventRequestFlags::FALLING_EDGE,
            "soundkid",
        ) {
            Ok(e) => e,
            Err(e) => {
                warn!("could not request events on GPIO {device:?}/{line_offset}: {e}");
                return;
            }
        };
        let mut events = match AsyncLineEventHandle::new(events) {
            Ok(e) => e,
            Err(e) => {
                warn!("could not wrap GPIO events as async on {device:?}/{line_offset}: {e}");
                return;
            }
        };
        info!("Watching GPIO line {line_offset} on {device}");
        while let Some(event) = events.next().await {
            match event {
                Ok(ev) => debug!("GPIO event on {device:?}/{line_offset}: {ev:?}"),
                Err(e) => {
                    warn!("GPIO event error on {device:?}/{line_offset}: {e}");
                    return;
                }
            }
            let msg = InputMessage {
                device_type: InputDeviceType::Gpio,
                device: device.clone(),
                id: line_offset.to_string(),
            };
            if tx.send(msg).await.is_err() {
                return;
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let _cli = Cli::parse();
    info!("Starting soundkid ...");

    let conf = Config::load().context("loading configuration")?;

    let (events_tx, events_rx) = mpsc::channel(100);

    if conf.input.is_empty() {
        info!("No input config found, skipping evdev handling");
    } else {
        for device_desc in conf.input.keys() {
            let reader = Input::new(device_desc)
                .with_context(|| format!("opening input device {device_desc:?}"))?;
            debug!("Got input reader {:?}", reader.device_desc);
            spawn_evdev_reader(reader, events_tx.clone());
        }
    }

    for (device, lines) in conf.gpio.clone() {
        info!("Found config for GPIO device {device}");
        for line in lines.keys() {
            spawn_gpio_reader(device.clone(), *line, events_tx.clone());
        }
    }

    let player = SpotifyPlayer::new(&conf.spotify)
        .await
        .context("setting up Spotify player")?;

    handle_input(conf, events_rx, player).await;
    Ok(())
}
