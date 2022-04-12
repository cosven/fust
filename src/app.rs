use crate::player::{PlayerMetadata, PlayerState, Progress};
use log::{error, info};
use std::io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::result::Result;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
pub struct Response {
    ok: bool,
    body: Vec<u8>,
}

pub struct Message {
    topic: String,
    body: Vec<u8>,
}

enum RespOrMsg {
    Response(Response),
    Message(Message),
}

fn read_response(reader: &mut BufReader<TcpStream>) -> Result<RespOrMsg, Error> {
    let mut status_line = String::new();
    if reader.read_line(&mut status_line).unwrap() == 0 {
        return Err(Error::new(ErrorKind::ConnectionAborted, "disconnected"));
    }
    let mut words = status_line.split_whitespace();
    let ack_or_msg = words.next().unwrap();
    let body_len_s = words.clone().last().unwrap();
    let body_len = body_len_s.parse::<usize>().unwrap();

    // Consume \r\n.
    let mut body = vec![0; body_len + 2];
    reader.read_exact(&mut body)?;
    body.truncate(body_len);

    // Response looks like::
    //   ACK OK 5
    //   hello
    // While message looks like::
    //   MSG topic_name 5
    //   hello
    if ack_or_msg.to_lowercase() == "ack" {
        let word = words.next().unwrap();
        let ok = word.to_lowercase() == *"ok";
        Ok(RespOrMsg::Response(Response { ok, body }))
    } else {
        let topic = words.next().unwrap().to_string();
        Ok(RespOrMsg::Message(Message { topic, body }))
    }
}

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

pub fn send_request(cmd: String) -> Result<Response, Error> {
    match TcpStream::connect("127.0.0.1:23333") {
        Ok(stream) => {
            info!("Successfully connected to fuo pubsub server in port 23333");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = BufWriter::new(stream);
            let mut line = String::new();

            // Receive the welcome message.
            if reader.read_line(&mut line).unwrap() > 0 {
                info!("{}", line);
            }

            writer.write_all(format!("{}\n", cmd).as_bytes()).unwrap();
            writer.flush().unwrap();
            match read_response(&mut reader)? {
                RespOrMsg::Response(resp) => Ok(resp),
                RespOrMsg::Message(_) => panic!(""),
            }
        }
        Err(e) => {
            log::error!("Failed to connect: {}", e);
            Err(e)
        }
    }
}

// TODO: exit and reconnect properly.
pub fn subscribe_signals(inner: Arc<Mutex<AppInner>>) {
    match TcpStream::connect("127.0.0.1:23334") {
        Ok(stream) => {
            info!("Successfully connected to fuo pubsub server in port 23334");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = BufWriter::new(stream);
            let mut line = String::new();

            // Receive the welcome message.
            if reader.read_line(&mut line).unwrap() > 0 {
                info!("{}", line);
            }

            // Subscribe topics and consume responses.
            writer
                .write_all(
                    "set --pubsub-version 2.0\n\
                     sub player.*\n\
                     sub live_lyric.*\n"
                        .as_bytes(),
                )
                .unwrap();
            writer.flush().unwrap();

            read_response(&mut reader).unwrap();
            read_response(&mut reader).unwrap();
            read_response(&mut reader).unwrap();

            // Wait for messages.
            loop {
                let resp_or_msg = read_response(&mut reader).unwrap();
                match resp_or_msg {
                    RespOrMsg::Message(msg) => inner.lock().unwrap().on_message(msg),
                    RespOrMsg::Response(_) => {}
                }
            }
        }
        Err(e) => {
            error!("Failed to connect: {}", e);
        }
    }
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
