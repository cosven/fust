// TODO: Maybe use slog.
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json;
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::convert::{TryFrom, TryInto};
use std::io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::result::Result;
use std::str::{from_utf8, SplitWhitespace};
use std::sync::{Arc, Mutex};
use std::thread;
use tui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Gauge, LineGauge},
    Frame,
};

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
    if reader.read_line(&mut status_line).unwrap() <= 0 {
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
        let ok = word.to_lowercase() == "ok".to_owned();
        Ok(RespOrMsg::Response(Response { ok: ok, body: body }))
    } else {
        let topic = words.next().unwrap().to_string();
        Ok(RespOrMsg::Message(Message {
            topic: topic,
            body: body,
        }))
    }
}

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
    title: String,
    artists: Vec<String>,
    album: Option<String>,
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

// Store app states.
#[allow(dead_code)]
pub struct AppInner {
    metadata: PlayerMetadata,
    position: u16,
    state: PlayerState,
}

impl AppInner {
    pub fn on_message(&mut self, msg: Message) {
        let body = String::from_utf8(msg.body.clone()).unwrap();
        match msg.topic.as_str() {
            "player.state_changed" => {
                // TODO: maybe use tuple?
                let value: serde_json::Value = serde_json::from_str(&body).unwrap();
                match value[0].as_u64().unwrap().try_into() {
                    Ok(state) => self.state = state,
                    Err(_) => panic!("unknown player state"),
                }
            }
            "player.metadata_changed" => {
                let args: (PlayerMetadata,) = serde_json::from_str(&body).unwrap();
                self.metadata = args.0;
            }
            _ => {}
        }
    }
}

pub struct App {
    inner: Arc<Mutex<AppInner>>,
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

            writer.write(format!("{}\n", cmd).as_bytes()).unwrap();
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

            writer
                .write("set --pubsub-version 2.0\n".as_bytes())
                .unwrap();
            writer.write("sub player.*\n".as_bytes()).unwrap();
            writer.flush().unwrap();
            // Consume two responses.
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
                position: 0,
                state: PlayerState::Stopped,
            })),
        }
    }

    pub fn on_tick(&mut self) {}

    pub fn sync_player_status(&mut self) {
        let resp = send_request("status --format=json".to_owned()).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        let song = value["song"].clone();
        match value["state"].as_str().unwrap() {
            "paused" => self.inner.lock().unwrap().state = PlayerState::Paused,
            "playing" => self.inner.lock().unwrap().state = PlayerState::Playing,
            "stopped" => self.inner.lock().unwrap().state = PlayerState::Stopped,
            _ => self.inner.lock().unwrap().state = PlayerState::Stopped,
        }
        // println!("{}", song["title"].to_string());
        // FIXME: there are quotes in the string.
        let metadata = PlayerMetadata {
            title: song["title"].to_string(),
            album: Some(song["album"]["name"].to_string()),
            artists: song["artists"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item: &serde_json::Value| item["name"].to_string())
                .collect::<Vec<String>>(),
        };
        self.inner.lock().unwrap().metadata = metadata.clone();
    }

    pub fn subscribe_msgs(&self) {
        let inner = self.inner.clone();
        thread::spawn(move || {
            subscribe_signals(inner);
        });
    }
}

//
// Code for UI.
//

fn s_player_state(state: PlayerState) -> String {
    match state {
        PlayerState::Stopped => "已停止".to_owned(),
        PlayerState::Paused => "暂停".to_owned(),
        PlayerState::Playing => "播放中".to_owned(),
    }
}

pub fn ui<B: Backend>(f: &mut Frame<B>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
        .split(f.size());

    let inner = app.inner.lock().unwrap();
    let metadata = inner.metadata.clone();
    let title = format!(
        " {}: {} - {} - {} ",
        s_player_state(inner.state),
        metadata.title,
        metadata.artists[0],
        metadata.album.unwrap_or_else(|| "".to_owned()),
    );
    drop(inner);
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .gauge_style(Style::default().fg(Color::Yellow))
        .percent(60);

    f.render_widget(gauge, chunks[0]);
}
