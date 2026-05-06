use anyhow::Result;
use tokio::process::Command;
use tokio::sync::mpsc::Receiver;
use tracing::{debug, info, warn};

use crate::config::{Action, Config};
use crate::input::{InputEvent, lookup_action};
use crate::player::PlayerControl;

/// Drive the dispatch loop: pull events off the channel, look up their
/// configured action, and invoke the player or amixer accordingly.
///
/// Returns when the channel closes (all senders dropped) or when a player
/// command fails — typically because the player task has died.
///
/// Generic over `PlayerControl` so tests can substitute a fake.
pub async fn handle_input<P: PlayerControl>(
    conf: Config,
    mut events_rx: Receiver<InputEvent>,
    player: P,
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
