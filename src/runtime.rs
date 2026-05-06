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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::PlayerControl;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    /// Records every command the dispatch loop sends. `play` can be
    /// configured to fail, mimicking a dead player task.
    #[derive(Debug, Clone, Default)]
    struct FakePlayer {
        log: Arc<Mutex<Vec<Cmd>>>,
        fail_play: Arc<Mutex<bool>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Cmd {
        Play(String),
        Stop,
        Pause,
        Resume,
    }

    impl FakePlayer {
        fn record(&self, cmd: Cmd) {
            self.log.lock().unwrap().push(cmd);
        }

        fn commands(&self) -> Vec<Cmd> {
            self.log.lock().unwrap().clone()
        }

        fn arm_play_failure(&self) {
            *self.fail_play.lock().unwrap() = true;
        }
    }

    impl PlayerControl for FakePlayer {
        async fn play(&self, uri: String) -> Result<()> {
            self.record(Cmd::Play(uri));
            if *self.fail_play.lock().unwrap() {
                Err(anyhow::anyhow!("simulated player failure"))
            } else {
                Ok(())
            }
        }

        async fn stop(&self) -> Result<()> {
            self.record(Cmd::Stop);
            Ok(())
        }

        async fn pause(&self) -> Result<()> {
            self.record(Cmd::Pause);
            Ok(())
        }

        async fn resume(&self) -> Result<()> {
            self.record(Cmd::Resume);
            Ok(())
        }
    }

    const TRACK: &str = "6rqhFgbbKwnb9MLmUQDhG6";

    fn config_yaml() -> String {
        format!(
            r#"
alsa: {{ control: "Master" }}
spotify: {{}}
input:
  /dev/input/event0:
    "PLAY_CARD": "spotify:track:{TRACK}"
    "PAUSE_CARD": "PAUSE"
    "RESUME_CARD": "RESUME"
    "VOL_UP_CARD": "VOLUME_INCREASE"
gpio:
  /dev/gpiochip0:
    17: "PAUSE"
"#
        )
    }

    fn build_config() -> Config {
        serde_yaml_ng::from_str(&config_yaml()).expect("test config must parse")
    }

    fn evdev(scanned: &str) -> InputEvent {
        InputEvent::Evdev {
            device: "/dev/input/event0".into(),
            scanned: scanned.into(),
        }
    }

    fn gpio(line: u32) -> InputEvent {
        InputEvent::Gpio {
            chip: "/dev/gpiochip0".into(),
            line,
        }
    }

    /// Drive handle_input to completion: send the events, drop the sender,
    /// await the dispatch loop.
    async fn run(events: Vec<InputEvent>, fake: FakePlayer) -> Result<()> {
        let conf = build_config();
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(handle_input(conf, rx, fake));
        for ev in events {
            tx.send(ev).await.unwrap();
        }
        drop(tx);
        task.await.unwrap()
    }

    #[tokio::test]
    async fn evdev_play_card_dispatches_play() {
        let fake = FakePlayer::default();
        run(vec![evdev("PLAY_CARD")], fake.clone()).await.unwrap();
        assert_eq!(
            fake.commands(),
            vec![Cmd::Play(format!("spotify:track:{TRACK}"))]
        );
    }

    #[tokio::test]
    async fn evdev_pause_card_dispatches_pause() {
        let fake = FakePlayer::default();
        run(vec![evdev("PAUSE_CARD")], fake.clone()).await.unwrap();
        assert_eq!(fake.commands(), vec![Cmd::Pause]);
    }

    #[tokio::test]
    async fn evdev_resume_card_dispatches_resume() {
        let fake = FakePlayer::default();
        run(vec![evdev("RESUME_CARD")], fake.clone()).await.unwrap();
        assert_eq!(fake.commands(), vec![Cmd::Resume]);
    }

    #[tokio::test]
    async fn volume_action_does_not_touch_player() {
        // amixer is unlikely to exist in CI, so it'll warn-and-continue;
        // either way no player command should be recorded.
        let fake = FakePlayer::default();
        run(vec![evdev("VOL_UP_CARD")], fake.clone()).await.unwrap();
        assert_eq!(fake.commands(), vec![]);
    }

    #[tokio::test]
    async fn unmapped_event_is_silently_ignored() {
        let fake = FakePlayer::default();
        run(vec![evdev("UNKNOWN_CARD")], fake.clone())
            .await
            .unwrap();
        assert_eq!(fake.commands(), vec![]);
    }

    #[tokio::test]
    async fn gpio_event_dispatches_via_gpio_table() {
        let fake = FakePlayer::default();
        run(vec![gpio(17)], fake.clone()).await.unwrap();
        assert_eq!(fake.commands(), vec![Cmd::Pause]);
    }

    #[tokio::test]
    async fn multiple_events_dispatch_in_order() {
        let fake = FakePlayer::default();
        run(
            vec![
                evdev("PLAY_CARD"),
                evdev("PAUSE_CARD"),
                evdev("RESUME_CARD"),
            ],
            fake.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            fake.commands(),
            vec![
                Cmd::Play(format!("spotify:track:{TRACK}")),
                Cmd::Pause,
                Cmd::Resume,
            ]
        );
    }

    #[tokio::test]
    async fn player_failure_stops_dispatch_loop() {
        let fake = FakePlayer::default();
        fake.arm_play_failure();
        let conf = build_config();
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(handle_input(conf, rx, fake.clone()));
        // Send a Play event that the fake will reject.
        tx.send(evdev("PLAY_CARD")).await.unwrap();
        // Even though we haven't dropped tx, the loop should exit on the
        // play() error. await its completion.
        let result = task.await.unwrap();
        assert!(result.is_err());
        // The Play was attempted and recorded before failure.
        assert_eq!(
            fake.commands(),
            vec![Cmd::Play(format!("spotify:track:{TRACK}"))]
        );
    }

    #[tokio::test]
    async fn empty_event_stream_returns_ok() {
        let fake = FakePlayer::default();
        run(vec![], fake.clone()).await.unwrap();
        assert!(fake.commands().is_empty());
    }
}
