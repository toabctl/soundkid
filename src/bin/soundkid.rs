use anyhow::{Context, Result, anyhow};
use clap::Parser;
use soundkid::{
    config::Config,
    player::SpotifyPlayer,
    reader::{Input, setup_gpio_line, spawn_evdev_reader, spawn_gpio_reader},
    runtime::handle_input,
};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tracing::{debug, info};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(name = "soundkid", version, about = "Sound player for kids")]
struct Cli {}

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
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
            let events = setup_gpio_line(&device, *line)
                .with_context(|| format!("setting up GPIO {device:?}/{line} from config"))?;
            spawn_gpio_reader(device.clone(), *line, events, events_tx.clone());
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
            Ok(()) => Err(anyhow!("player task exited unexpectedly")),
            Err(e) => Err(anyhow!("player task panicked: {e}")),
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
