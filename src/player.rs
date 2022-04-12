use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::convert::TryFrom;
use std::time::{Duration, SystemTime};

#[derive(Serialize_repr, Deserialize_repr, PartialEq, Debug, Copy, Clone)]
#[repr(u64)]
pub enum PlayerState {
    Stopped = 0,
    Paused = 1,
    Playing = 2,
}

impl TryFrom<u64> for PlayerState {
    type Error = ();

    fn try_from(v: u64) -> Result<Self, Self::Error> {
        match v {
            x if x == PlayerState::Stopped as u64 => Ok(PlayerState::Stopped),
            x if x == PlayerState::Paused as u64 => Ok(PlayerState::Paused),
            x if x == PlayerState::Playing as u64 => Ok(PlayerState::Playing),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlayerMetadata {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
}

impl PlayerMetadata {
    pub fn new() -> PlayerMetadata {
        PlayerMetadata {
            title: "".to_owned(),
            artists: vec!["".to_owned()],
            album: Some("".to_owned()),
        }
    }
}

pub struct Progress {
    ts: SystemTime,
    position: Duration,

    paused: bool,
    paused_ts: SystemTime,
}

impl Default for Progress {
    fn default() -> Progress {
        let now = SystemTime::now();
        Progress {
            ts: now,
            position: Duration::new(0, 0),
            paused_ts: now,
            paused: false,
        }
    }
}

impl Progress {
    pub fn on_seeked(&mut self, position: Duration) {
        self.ts = SystemTime::now();
        self.position = position;
    }

    pub fn pause(&mut self) {
        self.position = self.current();
        self.paused_ts = SystemTime::now();
        self.ts = self.paused_ts;
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.position = self.current();
        self.ts = SystemTime::now();
        self.paused = false;
    }

    pub fn current(&self) -> Duration {
        if self.paused {
            self.position
        } else {
            self.position + self.ts.elapsed().unwrap()
        }
    }
}
