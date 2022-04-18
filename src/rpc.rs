use crate::app::AppInner;
use log::{error, info};
use std::io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::result::Result;
use std::sync::{Arc, Mutex};

#[allow(dead_code)]
pub struct Response {
    pub ok: bool,
    pub body: Vec<u8>,
}

pub struct Message {
    pub topic: String,
    pub body: Vec<u8>,
}

pub enum RespOrMsg {
    Response(Response),
    Message(Message),
}

pub fn read_response(reader: &mut BufReader<TcpStream>) -> Result<RespOrMsg, Error> {
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
