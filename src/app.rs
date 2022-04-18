use crate::player::{PlayerMetadata, PlayerState, Progress};
use crate::rpc::{read_response, send_request, subscribe_signals, Message, RespOrMsg, Response};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// Store app states.
#[allow(dead_code)]
pub struct AppInner {
    pub metadata: PlayerMetadata,
    pub lyric_s: String, // Current lyric sentence.
    pub progress: Progress,
    pub duration: Duration,
    pub state: PlayerState,
}

impl AppInner {
    pub fn on_message(&mut self, msg: Message) {
        let body = String::from_utf8(msg.body.clone()).unwrap();
        match msg.topic.as_str() {
            "player.state_changed" => {
                // TODO: maybe use tuple?
                let value: serde_json::Value = serde_json::from_str(&body).unwrap();
                match value[0].as_u64().unwrap().try_into() {
                    Ok(state) => {
                        self.state = state;
                        match state {
                            PlayerState::Paused => self.progress.pause(),
                            PlayerState::Stopped => self.progress.on_seeked(Duration::new(0, 0)),
                            PlayerState::Playing => self.progress.resume(),
                        }
                    }
                    Err(_) => panic!("unknown player state"),
                }
            }
            "player.metadata_changed" => {
                let args: (PlayerMetadata,) = serde_json::from_str(&body).unwrap();
                self.metadata = args.0;
                self.progress.on_seeked(Duration::new(0, 0));
            }
            "player.duration_changed" => {
                let args: (f64,) = serde_json::from_str(&body).unwrap();
                self.duration = Duration::from_secs_f64(args.0 as f64);
            }
            "player.seeked" => {
                let args: (f64,) = serde_json::from_str(&body).unwrap();
                self.progress
                    .on_seeked(Duration::from_secs_f64(args.0 as f64));
            }
            "live_lyric.sentence_changed" => {
                if !body.is_empty() {
                    let args: (String,) = serde_json::from_str(&body).unwrap();
                    self.lyric_s = args.0;
                }
            }
            _ => {}
        }
    }
}

pub struct App {
    pub inner: Arc<Mutex<AppInner>>,
}

impl App {
    pub fn new() -> App {
        App {
            inner: Arc::new(Mutex::new(AppInner {
                metadata: PlayerMetadata::new(),
                lyric_s: "暂无歌词".to_owned(),
                progress: Progress::default(),
                duration: Duration::new(0, 0),
                state: PlayerState::Stopped,
            })),
        }
    }

    pub fn on_tick(&mut self) {}

    // Sync player status immediattely by sending a request `status --format=json`.
    pub fn sync_player_status(&mut self) {
        let resp = send_request("status --format=json".to_owned()).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        let song = value["song"].clone();
        let duration = Duration::from_secs_f64(value["duration"].as_f64().unwrap());
        let position = Duration::from_secs_f64(value["position"].as_f64().unwrap());
        let metadata = PlayerMetadata {
            title: song["title"].as_str().unwrap().to_string(),
            album: Some(song["album_name"].as_str().unwrap().to_string()),
            artists: vec![song["artists_name"].as_str().unwrap().to_string()],
        };
        {
            let mut inner = self.inner.lock().unwrap();
            inner.metadata = metadata;
            inner.progress.on_seeked(position);
            inner.duration = duration;
            match value["state"].as_str().unwrap() {
                "paused" => {
                    inner.state = PlayerState::Paused;
                    inner.progress.pause();
                }
                "stopped" => {
                    inner.state = PlayerState::Stopped;
                    inner.progress.pause();
                }
                "playing" => {
                    inner.state = PlayerState::Playing;
                    inner.progress.resume();
                }
                _ => self.inner.lock().unwrap().state = PlayerState::Stopped,
            }
        }
    }

    pub fn subscribe_msgs(&self) {
        let inner = self.inner.clone();
        thread::spawn(move || {
            subscribe_signals(inner);
        });
    }
}
