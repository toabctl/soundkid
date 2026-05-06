//! End-to-end integration tests for the YAML → InputEvent → Action → Player
//! dispatch flow. These exercise only soundkid's public API.

mod common;

use common::{Cmd, FakePlayer};
use soundkid::{config::Config, input::InputEvent, runtime::handle_input};
use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;

const TRACK: &str = "6rqhFgbbKwnb9MLmUQDhG6";
const ALBUM: &str = "7LQhG0xSDjFiKJnziyB3Zj";

fn write_temp(yaml: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    f
}

async fn load_yaml(yaml: &str) -> Config {
    let f = write_temp(yaml);
    Config::load_from(vec![f.path().to_path_buf()])
        .await
        .expect("test config must load")
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

/// Drive handle_input to completion: spawn it, send events, drop the sender,
/// await.
async fn run_dispatch(
    conf: Config,
    fake: FakePlayer,
    events: Vec<InputEvent>,
) -> Result<(), soundkid::player::PlayerError> {
    let (tx, rx) = mpsc::channel(8);
    let task = tokio::spawn(handle_input(conf, rx, fake));
    for ev in events {
        tx.send(ev).await.unwrap();
    }
    drop(tx);
    task.await.unwrap()
}

#[tokio::test]
async fn yaml_to_dispatch_happy_path() {
    let yaml = format!(
        r#"
alsa: {{ control: "Master" }}
spotify: {{}}
input:
  /dev/input/event0:
    "PLAY_CARD": "spotify:track:{TRACK}"
"#
    );
    let conf = load_yaml(&yaml).await;
    let fake = FakePlayer::new();
    run_dispatch(conf, fake.clone(), vec![evdev("PLAY_CARD")])
        .await
        .unwrap();
    assert_eq!(
        fake.commands(),
        vec![Cmd::Play(format!("spotify:track:{TRACK}"))]
    );
}

#[tokio::test]
async fn url_in_yaml_canonicalised_at_load_time() {
    // The user types an https://open.spotify.com URL in the config; by the
    // time we dispatch the matching card, it's already in spotify: form.
    let yaml = format!(
        r#"
alsa: {{}}
spotify: {{}}
input:
  /dev/input/event0:
    "ALBUM_CARD": "https://open.spotify.com/album/{ALBUM}?si=foo"
"#
    );
    let conf = load_yaml(&yaml).await;
    let fake = FakePlayer::new();
    run_dispatch(conf, fake.clone(), vec![evdev("ALBUM_CARD")])
        .await
        .unwrap();
    assert_eq!(
        fake.commands(),
        vec![Cmd::Play(format!("spotify:album:{ALBUM}"))]
    );
}

#[tokio::test]
async fn typo_in_action_keyword_rejected_at_load() {
    let f = write_temp(
        r#"
alsa: {}
spotify: {}
input:
  /dev/input/event0:
    "BAD": "VOLUME_INCREASS"
"#,
    );
    let err = Config::load_from(vec![f.path().to_path_buf()])
        .await
        .unwrap_err();
    let chain = error_chain(&err);
    assert!(
        chain.contains("VOLUME_INCREASS"),
        "error chain should name the bad value: {chain}"
    );
}

#[tokio::test]
async fn short_spotify_id_rejected_at_load() {
    let f = write_temp(
        r#"
alsa: {}
spotify: {}
input:
  /dev/input/event0:
    "BAD": "spotify:track:abc"
"#,
    );
    assert!(
        Config::load_from(vec![f.path().to_path_buf()])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn config_precedence_first_wins() {
    let primary = write_temp(
        r#"
alsa: { control: "PrimaryMaster" }
spotify: {}
"#,
    );
    let secondary = write_temp(
        r#"
alsa: { control: "SecondaryMaster" }
spotify: {}
"#,
    );
    let conf = Config::load_from(vec![
        primary.path().to_path_buf(),
        secondary.path().to_path_buf(),
    ])
    .await
    .unwrap();
    assert_eq!(conf.alsa.control, "PrimaryMaster");
}

#[tokio::test]
async fn config_falls_back_when_primary_missing() {
    let secondary = write_temp(
        r#"
alsa: { control: "Fallback" }
spotify: {}
"#,
    );
    let conf = Config::load_from(vec![
        PathBuf::from("/no/such/file.conf"),
        secondary.path().to_path_buf(),
    ])
    .await
    .unwrap();
    assert_eq!(conf.alsa.control, "Fallback");
}

#[tokio::test]
async fn unmapped_event_does_not_dispatch() {
    let conf = load_yaml(
        r#"
alsa: {}
spotify: {}
"#,
    )
    .await;
    let fake = FakePlayer::new();
    run_dispatch(conf, fake.clone(), vec![evdev("UNKNOWN_CARD")])
        .await
        .unwrap();
    assert!(fake.commands().is_empty());
}

#[tokio::test]
async fn multi_event_sequence_dispatches_in_order() {
    let yaml = format!(
        r#"
alsa: {{}}
spotify: {{}}
input:
  /dev/input/event0:
    "PLAY": "spotify:track:{TRACK}"
    "PAUSE_IT": "PAUSE"
    "RESUME_IT": "RESUME"
"#
    );
    let conf = load_yaml(&yaml).await;
    let fake = FakePlayer::new();
    run_dispatch(
        conf,
        fake.clone(),
        vec![evdev("PLAY"), evdev("PAUSE_IT"), evdev("RESUME_IT")],
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
async fn gpio_event_dispatches_via_gpio_table() {
    let yaml = r#"
alsa: {}
spotify: {}
gpio:
  /dev/gpiochip0:
    17: "PAUSE"
"#;
    let conf = load_yaml(yaml).await;
    let fake = FakePlayer::new();
    run_dispatch(conf, fake.clone(), vec![gpio(17)])
        .await
        .unwrap();
    assert_eq!(fake.commands(), vec![Cmd::Pause]);
}

#[tokio::test]
async fn player_failure_short_circuits_dispatch() {
    let yaml = format!(
        r#"
alsa: {{}}
spotify: {{}}
input:
  /dev/input/event0:
    "PLAY": "spotify:track:{TRACK}"
    "PAUSE_IT": "PAUSE"
"#
    );
    let conf = load_yaml(&yaml).await;
    let fake = FakePlayer::new();
    fake.arm_play_failure();

    let (tx, rx) = mpsc::channel(8);
    let task = tokio::spawn(handle_input(conf, rx, fake.clone()));
    tx.send(evdev("PLAY")).await.unwrap();
    // Don't drop tx; the loop should exit on the play() error before the
    // second event even gets dispatched.
    let _ = tx.try_send(evdev("PAUSE_IT"));
    let res = task.await.unwrap();
    assert!(res.is_err());

    // First (failing) Play recorded; subsequent events are not dispatched.
    assert_eq!(
        fake.commands(),
        vec![Cmd::Play(format!("spotify:track:{TRACK}"))]
    );
}

#[tokio::test]
async fn empty_event_stream_returns_ok() {
    let conf = load_yaml(
        r#"
alsa: {}
spotify: {}
"#,
    )
    .await;
    let fake = FakePlayer::new();
    run_dispatch(conf, fake.clone(), vec![]).await.unwrap();
    assert!(fake.commands().is_empty());
}

/// Walk the std::error::Error source chain into a single string for assertion.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut messages = Vec::new();
    let mut current: Option<&dyn std::error::Error> = Some(err);
    while let Some(e) = current {
        messages.push(e.to_string());
        current = e.source();
    }
    messages.join(" / ")
}
