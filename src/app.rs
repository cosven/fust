// TODO: Maybe use slog.
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json;
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::cmp::min;
use std::convert::{TryFrom, TryInto};
use std::io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::result::Result;
use std::str::{from_utf8, SplitWhitespace};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use tui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    symbols::line::THICK,
    symbols::DOT,
    text::{Span, Spans},
    widgets::{Block, Borders, Gauge, LineGauge, Paragraph, Wrap},
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

fn fmt_duration(duration: Duration) -> String {
    let seconds = duration.as_secs() % 60;
    let minutes = (duration.as_secs() / 60) % 60;
    let hours = (duration.as_secs() / 60) / 60;
    if hours > 0 {
        format!("{:0>2}:{:0>2}:{:0>2}", hours, minutes, seconds)
    } else {
        format!("{:0>2}:{:0>2}", minutes, seconds)
    }
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

pub struct Progress {
    ts: SystemTime,
    position: Duration,

    paused: bool,
    paused_ts: SystemTime,
}

impl Default for Progress {
    fn default() -> Progress {
        let now = SystemTime::now();
        return Progress {
            ts: now,
            position: Duration::new(0, 0),

            paused_ts: now,
            paused: false,
        };
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

// Store app states.
#[allow(dead_code)]
pub struct AppInner {
    metadata: PlayerMetadata,
    lyric_s: String, // Current lyric sentence.
    progress: Progress,
    duration: Duration,
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
                if body.len() > 0 {
                    let args: (String,) = serde_json::from_str(&body).unwrap();
                    self.lyric_s = args.0;
                }
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

            // Subscribe topics and consume responses.
            writer
                .write("set --pubsub-version 2.0\n".as_bytes())
                .unwrap();
            writer.write("sub player.*\n".as_bytes()).unwrap();
            writer.write("sub live_lyric.*\n".as_bytes()).unwrap();
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
            inner.metadata = metadata.clone();
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

//
// Code for UI.
//

pub fn ui<B: Backend>(f: &mut Frame<B>, app: &App) {
    let area = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ]
            .as_ref(),
        )
        .split(area);

    let inner = app.inner.lock().unwrap();
    let metadata = inner.metadata.clone();
    let lyric_s = inner.lyric_s.clone();
    let position = inner.progress.current();
    let duration = inner.duration;
    let state = inner.state;
    drop(inner);

    let mut song_spans = vec![
        Span::raw(" ".to_owned()),
        Span::styled("♫  ", Style::default().fg(Color::Yellow)),
        Span::raw(metadata.title),
    ];
    if metadata.artists.len() > 0 {
        song_spans.push(Span::raw(DOT));
        song_spans.push(Span::styled(DOT, Style::default().fg(Color::Gray)));
        song_spans.push(Span::raw(metadata.artists.join(",")));
    }

    let color = match state {
        PlayerState::Stopped => Color::Gray,
        PlayerState::Paused => Color::Gray,
        PlayerState::Playing => Color::LightCyan,
    };
    let ratio = match duration.as_secs_f64() <= 0.0 {
        true => 0.0,
        false => {
            let ratio = position.as_secs_f64() / duration.as_secs_f64();
            if ratio >= 1.0 {
                1.0 as f64
            } else {
                ratio
            }
        }
    };
    let progress = LineGauge::default()
        .gauge_style(Style::default().fg(color))
        .label(Span::styled(
            format!("[{}/{}]", fmt_duration(position), fmt_duration(duration)),
            Style::default().fg(color).add_modifier(Modifier::ITALIC),
        ))
        .line_set(THICK)
        .ratio(ratio);
    f.render_widget(progress, chunks[2]);

    let lyric = Paragraph::new(vec![Spans::from(lyric_s.to_owned())])
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Right);
    let song = Paragraph::new(Spans::from(song_spans)).wrap(Wrap { trim: true });
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .margin(0)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(chunks[3]);
    f.render_widget(song, h_chunks[0]);
    f.render_widget(lyric, h_chunks[1]);
}
