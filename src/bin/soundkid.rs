use anyhow::{Context, Result};
use clap::Parser;
use evdev::{EventSummary, KeyCode};
use futures::stream::StreamExt;
use gpio_cdev::{AsyncLineEventHandle, Chip, EventRequestFlags, LineRequestFlags};
use log::{debug, info, warn};
use soundkid::{
    config::{Action, Config},
    player::SpotifyPlayer,
    reader::Input,
};
use tokio::process::Command;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(name = "soundkid", version, about = "Sound player for kids")]
struct Cli {}

#[derive(Debug, Clone)]
enum InputEvent {
    Evdev { device: String, scanned: String },
    Gpio { chip: String, line: u32 },
}

fn lookup_action<'a>(conf: &'a Config, ev: &InputEvent) -> Option<&'a Action> {
    match ev {
        InputEvent::Evdev { device, scanned } => {
            conf.input.get(device).and_then(|m| m.get(scanned))
        }
        InputEvent::Gpio { chip, line } => conf.gpio.get(chip).and_then(|m| m.get(line)),
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
    mut events_rx: mpsc::Receiver<InputEvent>,
    player: SpotifyPlayer,
) -> Result<()> {
    info!("Input receiver started");
    while let Some(event) = events_rx.recv().await {
        debug!("Received {event:?}");
        let Some(action) = lookup_action(&conf, &event) else {
            warn!("no action configured for {event:?}");
            continue;
        };
        info!("Dispatching {action:?} from {event:?}");
        match action {
            Action::VolumeIncrease => amixer(&conf.alsa.control, "5%+").await,
            Action::VolumeDecrease => amixer(&conf.alsa.control, "5%-").await,
            Action::Pause => player.pause().await?,
            Action::Resume => player.resume().await?,
            Action::Play(uri) => player.play(uri.clone()).await?,
        }
    }
    Ok(())
}

fn spawn_evdev_reader(reader: Input, tx: mpsc::Sender<InputEvent>) {
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

fn spawn_gpio_reader(chip_path: String, line: u32, tx: mpsc::Sender<InputEvent>) {
    tokio::spawn(async move {
        let mut chip = match Chip::new(&chip_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("could not open GPIO chip {chip_path:?}: {e}");
                return;
            }
        };
        let chip_line = match chip.get_line(line) {
            Ok(l) => l,
            Err(e) => {
                warn!("could not get GPIO line {line} on {chip_path:?}: {e}");
                return;
            }
        };
        let events = match chip_line.events(
            LineRequestFlags::INPUT,
            EventRequestFlags::FALLING_EDGE,
            "soundkid",
        ) {
            Ok(e) => e,
            Err(e) => {
                warn!("could not request events on GPIO {chip_path:?}/{line}: {e}");
                return;
            }
        };
        let mut events = match AsyncLineEventHandle::new(events) {
            Ok(e) => e,
            Err(e) => {
                warn!("could not wrap GPIO events as async on {chip_path:?}/{line}: {e}");
                return;
            }
        };
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

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let _cli = Cli::parse();
    info!("Starting soundkid ...");

    let conf = Config::load().await.context("loading configuration")?;

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

    let (player, mut player_join) = SpotifyPlayer::new(&conf.spotify)
        .await
        .context("setting up Spotify player")?;

    let mut sigterm = signal(SignalKind::terminate()).context("install SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("install SIGINT handler")?;

    let result = tokio::select! {
        result = handle_input(conf, events_rx, player.clone()) => result,
        join = &mut player_join => match join {
            Ok(()) => Err(anyhow::anyhow!("player task exited unexpectedly")),
            Err(e) => Err(anyhow::anyhow!("player task panicked: {e}")),
        },
        _ = sigterm.recv() => {
            info!("SIGTERM received, shutting down");
            Ok(())
        }
        _ = sigint.recv() => {
            info!("SIGINT received, shutting down");
            Ok(())
        }
    };

    // Best-effort: silence the speaker before tearing down the runtime so we
    // don't leave a half-decoded buffer in the audio pipeline.
    let _ = player.stop().await;
    result
}
