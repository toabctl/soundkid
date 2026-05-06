//! Shared helpers for integration tests.
//!
//! `FakePlayer` is a `PlayerControl` that records every command into a
//! `Mutex<Vec<Cmd>>`. `arm_play_failure` makes the next `play()` call
//! return `PlayerError::Closed`, mimicking a dead player task.

use soundkid::player::{PlayerControl, PlayerError};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Play(String),
    Stop,
    Pause,
    Resume,
}

#[derive(Debug, Clone, Default)]
pub struct FakePlayer {
    log: Arc<Mutex<Vec<Cmd>>>,
    fail_play: Arc<Mutex<bool>>,
}

#[allow(dead_code)] // Some helpers only used by a subset of test files.
impl FakePlayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn commands(&self) -> Vec<Cmd> {
        self.log.lock().unwrap().clone()
    }

    pub fn arm_play_failure(&self) {
        *self.fail_play.lock().unwrap() = true;
    }

    fn record(&self, cmd: Cmd) {
        self.log.lock().unwrap().push(cmd);
    }
}

impl PlayerControl for FakePlayer {
    async fn play(&self, uri: String) -> Result<(), PlayerError> {
        self.record(Cmd::Play(uri));
        if *self.fail_play.lock().unwrap() {
            Err(PlayerError::Closed)
        } else {
            Ok(())
        }
    }

    async fn stop(&self) -> Result<(), PlayerError> {
        self.record(Cmd::Stop);
        Ok(())
    }

    async fn pause(&self) -> Result<(), PlayerError> {
        self.record(Cmd::Pause);
        Ok(())
    }

    async fn resume(&self) -> Result<(), PlayerError> {
        self.record(Cmd::Resume);
        Ok(())
    }
}
