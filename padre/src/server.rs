//! handle server connections

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use crate::debugger::PadreDebugger;
use crate::notifier::{LogLevel, Notifier};
use crate::request::{PadreRequest, PadreRequestCmd, PadreResponse};

use bytes::{BufMut, BytesMut};
use tokio::codec::{Decoder, Encoder};
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::sync::mpsc;

pub fn process_connection(
    socket: TcpStream,
    debugger: Arc<Mutex<PadreDebugger>>,
    notifier: Arc<Mutex<Notifier>>,
) {
    let addr = socket.peer_addr().unwrap();

    let (request_tx, request_rx) = PadreCodec::new(notifier.clone()).framed(socket).split();

    let (connection_tx, connection_rx) = mpsc::channel(1);

    notifier
        .lock()
        .unwrap()
        .add_listener(connection_tx.clone(), addr);

    if debugger.lock().unwrap().has_started() {
        notifier.lock().unwrap().signal_started();
    }

    tokio::spawn(
        request_tx
            .send_all(connection_rx.map_err(|e| {
                eprintln!("failed to retrieve message to send: {}", e);
                io::Error::new(io::ErrorKind::Other, e)
            }))
            .then(|res| {
                if let Err(e) = res {
                    eprintln!("failed to send data to socket; error = {:?}", e);
                }

                Ok(())
            }),
    );

    tokio::spawn(
        request_rx
            .and_then(move |req| {
                let debugger = debugger.clone();
                respond(req, debugger)
            })
            .for_each(move |resp| {
                tokio::spawn(
                    connection_tx
                        .clone()
                        .send(resp)
                        .map(|_| {})
                        .map_err(|e| println!("Error responding: {}", e)),
                );
                Ok(())
            })
            .map_err(|e| {
                eprintln!("failed to accept socket; error = {:?}", e);
            }),
    );
}

fn respond(
    req: PadreRequest,
    debugger: Arc<Mutex<PadreDebugger>>,
) -> Box<dyn Future<Item = PadreResponse, Error = io::Error> + Send> {
    let json_response = match req.cmd() {
        PadreRequestCmd::Cmd(s) => {
            let s: &str = s;
            match s {
                "ping" => debugger.lock().unwrap().ping(),
                "pings" => debugger.lock().unwrap().pings(),
                _ => return respond_debugger(req, debugger),
            }
        }
        _ => return respond_debugger(req, debugger),
    };

    let f = future::lazy(move || match json_response {
        Ok(resp) => Ok(PadreResponse::Response(req.id(), resp)),
        Err(_) => {
            unreachable!();
        }
    });

    Box::new(f)
}

fn respond_debugger(
    req: PadreRequest,
    debugger: Arc<Mutex<PadreDebugger>>,
) -> Box<dyn Future<Item = PadreResponse, Error = io::Error> + Send> {
    let id = req.id();

    // TODO: Timeouts
    let f = debugger
        .lock()
        .unwrap()
        .handle(req)
        .then(move |resp| Ok(PadreResponse::Response(id, resp.unwrap())));

    return Box::new(f);
}

#[derive(Debug)]
struct PadreCodec {
    // Track a list of places we should try from in case one of the sends cut off
    notifier: Arc<Mutex<Notifier>>,
}

impl PadreCodec {
    fn new(notifier: Arc<Mutex<Notifier>>) -> Self {
        PadreCodec { notifier }
    }

    fn send_error_and_debug(
        &self,
        err_msg: String,
        debug_msg: String,
    ) -> Result<Option<PadreRequest>, io::Error> {
        self.notifier.lock().unwrap().log_msg(LogLevel::ERROR, err_msg);
        self.notifier.lock().unwrap().log_msg(LogLevel::DEBUG, debug_msg);
        Ok(None)
    }
}

impl Decoder for PadreCodec {
    type Item = PadreRequest;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() == 0 {
            return Ok(None);
        }

        let mut stream = serde_json::Deserializer::from_slice(src).into_iter::<serde_json::Value>();
        let req = src.clone();

        let mut v = match stream.next() {
            Some(s) => match s {
                Ok(t) => t,
                Err(e) => {
                    match e.classify() {
                        serde_json::error::Category::Io => {
                            println!("IO: {:?}", req);
                        },
                        serde_json::error::Category::Syntax => {},
                        serde_json::error::Category::Data => {
                            println!("Data: {:?}", req);
                        },
                        serde_json::error::Category::Eof => {
                            return Ok(None);
                        },
                    };

                    src.split_to(src.len());

                    return self.send_error_and_debug(
                        "Must be valid JSON".to_string(),
                        format!("Can't read '{}': {}", String::from_utf8_lossy(&req[..]).trim_matches(char::from(0)), e),
                    );
                },
            },
            None => {
                unreachable!("HEREEEE2");
            }
        };

        src.split_to(src.len());

        if !v.is_array() {
            return self.send_error_and_debug(
                "Can't read JSON".to_string(),
                format!("Can't read '{}': Must be an array", String::from_utf8_lossy(&req[..]).trim_matches(char::from(0))),
            );
        }

        if v.as_array().unwrap().len() != 2 {
            return self.send_error_and_debug(
                "Can't read JSON".to_string(),
                format!("Can't read '{}': Array should have 2 elements", String::from_utf8_lossy(&req[..]).trim_matches(char::from(0))),
            );
        }

        let id = v[0].take();
        let id: u64 = match serde_json::from_value(id.clone()) {
            Ok(s) => s,
            Err(e) => {
                return self.send_error_and_debug(
                    "Can't read id".to_string(),
                    format!("Can't read '{}': {}", id, e),
                );
            }
        };

        let mut args: HashMap<String, serde_json::Value> = match serde_json::from_str(&v[1].take().to_string()) {
            Ok(args) => args,
            Err(e) => {
                return self.send_error_and_debug(
                    "Can't read JSON".to_string(),
                    format!("Can't read '{}': {}", String::from_utf8_lossy(&req[..]).trim_matches(char::from(0)), e),
                );
            }
        };

        let cmd = match args.remove("cmd") {
            Some(s) => s,
            None => {
                return self.send_error_and_debug(
                    "Can't find command".to_string(),
                    format!("Can't find command '{}': Need a cmd in 2nd object", String::from_utf8_lossy(&req[..]).trim_matches(char::from(0))),
                );
            }
        };

        let cmd: String = match serde_json::from_value(cmd) {
            Ok(s) => s,
            Err(e) => {
                return self.send_error_and_debug(
                    "Can't find command".to_string(),
                    format!("Can't find command '{}': {}", String::from_utf8_lossy(&req[..]).trim_matches(char::from(0)), e),
                );
            }
        };

        let file_location: Option<(String, u64)> = match args.remove("file") {
            Some(s) => {
                match s {
                    serde_json::Value::String(s) => {
                        match args.remove("line") {
                            Some(t) => {
                                match t {
                                    serde_json::Value::Number(t) => {
                                        let t: u64 = match t.as_u64() {
                                            Some(t) => t,
                                            None => {
                                                return self.send_error_and_debug(
                                                    format!("Badly specified 'line'"),
                                                    format!("Badly specified 'line': {}", t),
                                                );
                                            }
                                        };
                                        Some((s, t))
                                    }
                                    _ => {
                                        return self.send_error_and_debug(
                                            "Can't read 'line' argument".to_string(),
                                            format!("Can't understand 'line': {}", t),
                                        );
                                    }
                                }
                            }
                            None => {
                                return self.send_error_and_debug(
                                    "Can't read 'line' for file location when 'file' specified".to_string(),
                                    format!("Can't understand command with file but no line: '{}'", cmd),
                                );
                            }
                        }
                    }
                    _ => {
                        return self.send_error_and_debug(
                            format!("Can't read 'file' argument"),
                            format!("Can't understand 'file': {}", s),
                        );
                    }
                }
            },
            None => {
                None
            }
        };

        let variable: Option<String> = match args.remove("variable") {
            Some(s) => {
                match s {
                    serde_json::Value::String(s) => {
                        Some(s)
                    },
                    _ => {
                        panic!("ERROR2");
                    }
                }
            },
            None => {
                None
            }
        };

        let cmd: PadreRequestCmd = match file_location {
            Some(s) => PadreRequestCmd::CmdWithFileLocation(cmd, s.0, s.1),
            None => match variable {
                Some(s) => PadreRequestCmd::CmdWithVariable(cmd, s),
                None => PadreRequestCmd::Cmd(cmd),
            },
        };

        if !args.is_empty() {
            let mut args_left: Vec<String> = args.iter().map(|(key, _)| key.clone()).collect();
            args_left.sort();
            return self.send_error_and_debug(
                "Bad arguments".to_string(),
                format!("Bad arguments: {:?}", args_left),
            );
        }

        let padre_request: PadreRequest = PadreRequest::new(id, cmd);
        // TODO: If anything left in v error

        Ok(Some(padre_request))
    }
}

impl Encoder for PadreCodec {
    type Item = PadreResponse;
    type Error = io::Error;

    fn encode(&mut self, resp: PadreResponse, buf: &mut BytesMut) -> Result<(), io::Error> {
        let response = match resp {
            PadreResponse::Response(id, json) => serde_json::to_string(&(id, json)).unwrap(),
            PadreResponse::Notify(cmd, args) => {
                serde_json::to_string(&("call".to_string(), cmd, args)).unwrap()
            }
        };

        buf.reserve(response.len());
        buf.put(&response[..]);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::request::{PadreRequest, PadreRequestCmd, PadreResponse};
    use bytes::{BufMut, Bytes, BytesMut};
    use tokio::codec::{Decoder, Encoder};

    #[test]
    fn check_simple_json_decoding() {
        let mut codec = super::PadreCodec::new();
        let mut buf = BytesMut::new();
        buf.reserve(19);
        buf.put(r#"[123,{"cmd":"run"}]"#);

        let padre_request = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(
            PadreRequest::new(123, PadreRequestCmd::Cmd("run".to_string())),
            padre_request
        );
    }

    #[test]
    fn check_two_json_decodings() {
        let mut codec = super::PadreCodec::new();
        let mut buf = BytesMut::new();
        buf.reserve(19);
        buf.put(r#"[123,{"cmd":"run"}]"#);

        let padre_request = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(
            PadreRequest::new(123, PadreRequestCmd::Cmd("run".to_string())),
            padre_request
        );

        buf.reserve(19);
        buf.put(r#"[124,{"cmd":"run"}]"#);

        let padre_request = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(
            PadreRequest::new(124, PadreRequestCmd::Cmd("run".to_string())),
            padre_request
        );
    }

    #[test]
    fn check_two_buffers_json_decodings() {
        let mut codec = super::PadreCodec::new();
        let mut buf = BytesMut::new();
        buf.reserve(16);
        buf.put(r#"[123,{"cmd":"run"#);

        let padre_request = codec.decode(&mut buf).unwrap();

        assert_eq!(None, padre_request);

        buf.reserve(3);
        buf.put(r#""}]"#);

        let padre_request = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(
            PadreRequest::new(123, PadreRequestCmd::Cmd("run".to_string())),
            padre_request
        );
    }

    #[test]
    fn check_bad_then_good_json_decodings() {
        let mut codec = super::PadreCodec::new();
        let mut buf = BytesMut::new();
        buf.reserve(16);
        buf.put(r#"[123,{"cmd":"run"#);

        let padre_request = codec.decode(&mut buf).unwrap();

        assert_eq!(None, padre_request);

        buf.reserve(19);
        buf.put(r#"[124,{"cmd":"run"}]"#);

        let padre_request = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(
            PadreRequest::new(124, PadreRequestCmd::Cmd("run".to_string())),
            padre_request
        );
    }

    #[test]
    fn check_json_decoding_with_file_location() {
        let mut codec = super::PadreCodec::new();
        let mut buf = BytesMut::new();
        buf.reserve(53);
        buf.put(r#"[123,{"cmd":"breakpoint","file":"test.c","line":125}]"#);

        let padre_request = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(
            PadreRequest::new(
                123,
                PadreRequestCmd::CmdWithFileLocation(
                    "breakpoint".to_string(),
                    "test.c".to_string(),
                    125
                )
            ),
            padre_request
        );
    }

    #[test]
    fn check_json_decoding_with_variable() {
        let mut codec = super::PadreCodec::new();
        let mut buf = BytesMut::new();
        buf.reserve(36);
        buf.put(r#"[123,{"cmd":"print","variable":"a"}]"#);

        let padre_request = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(
            PadreRequest::new(
                123,
                PadreRequestCmd::CmdWithVariable("print".to_string(), "a".to_string())
            ),
            padre_request
        );
    }

    #[test]
    fn check_json_encoding_response() {
        let mut codec = super::PadreCodec::new();
        let resp = PadreResponse::Response(123, serde_json::json!({"ping":"pong"}));
        let mut buf = BytesMut::new();
        codec.encode(resp, &mut buf);

        let mut expected = BytesMut::new();
        expected.reserve(21);
        expected.put(r#"[123,{"ping":"pong"}]"#);

        assert_eq!(expected, buf);
    }

    #[test]
    fn check_json_encoding_notify() {
        let mut codec = super::PadreCodec::new();
        let resp = PadreResponse::Notify(
            "cmd_test".to_string(),
            vec![serde_json::json!("test"), serde_json::json!(1)],
        );
        let mut buf = BytesMut::new();
        codec.encode(resp, &mut buf);

        let mut expected = BytesMut::new();
        expected.reserve(32);
        expected.put(r#"["call","cmd_test",["test",1]]"#);

        assert_eq!(expected, buf);
    }
}
