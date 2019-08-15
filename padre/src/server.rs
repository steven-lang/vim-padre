//! Server
//!
//! Handles the main network connections, parses basic messages and forwards to
//! padre and debuggers for actioning.

use std::io;

use crate::notifier::{add_listener, remove_listener, log_msg, LogLevel};
use crate::vimcodec::VimCodec;

use tokio::codec::Decoder;
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::sync::mpsc;

// TODO: Get some of this out of pub use and just in this module?

/// File location
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct FileLocation {
    line_num: u32,
    file_name: String,
}

/// Variable name
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct Variable {
    variable_name: String,
}

/// All padre commands
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub enum PadreCmd {
    Ping,
    Pings,
    SetConfig, // TODO: (HashMap<>),
}

/// All debugger commands
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub enum DebuggerCmd {
    Run,
    Breakpoint(FileLocation),
    StepIn,
    StepOver,
    Continue,
    Variable(Variable),
}

/// Contains command details of a request, either a `PadreCmd` or a `DebuggerCmd`
///
/// Can be of the form of a command without arguments, a command with a location argument or a
/// command with a variable argument.
///
/// Examples:
///
/// ```
/// let command = RequestCmd::Cmd("run")
/// let command = RequestCmd::CmdWithFileLocation("breakpoint", "test.c", 12)
/// let command = RequestCmd::CmdWithVariable("print", "abc")
/// ```
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub enum RequestCmd {
    RequestPadreCmd(PadreCmd),
    RequestDebuggerCmd(DebuggerCmd),
}

/// Contains full details of a request including an id to respond to and a `RequestCmd`
#[derive(Deserialize, Debug, PartialEq)]
pub struct PadreRequest {
    id: u64,
    cmd: RequestCmd,
}

impl PadreRequest {
    /// Create a request
    pub fn new(id: u64, cmd: RequestCmd) -> Self {
        PadreRequest { id, cmd }
    }

    /// Return the request id
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Return the RequestCmd entry
    pub fn cmd(&self) -> &RequestCmd {
        &self.cmd
    }
}

/// A response to a request
///
/// Takes a u64 as the first argument to represent the id and a JSON object as
/// the second argument to represent the response. For example a response with an id of `1`
/// and a JSON object of `{"status":"OK"}` will be decoded by the `VIMCodec` as
/// `[1,{"status":"OK"}]` and sent as a response to the requesting socket.
///
/// Normally kept simple with important information relegated to an event based notification.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Response {
    id: u64,
    resp: serde_json::Value,
}

impl Response {
    /// Create a response
    pub fn new(id: u64, resp: serde_json::Value) -> Self {
        Response { id, resp }
    }

    /// Return the response id
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Return the response values
    pub fn resp(&self) -> &serde_json::Value {
        &self.resp
    }
}

/// A notification to be sent to all listeners of an event
///
/// Takes a String as the command and a vector of JSON values as arguments. For example, a
/// `Notication` with a command `call` and vector arguments TODO...
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Notification {
    cmd: String,
    args: Vec<serde_json::Value>,
}

impl Notification {
    /// Create a notification
    pub fn new(cmd: String, args: Vec<serde_json::Value>) -> Self {
        Notification { cmd, args }
    }

    /// Return the notification cmd
    pub fn cmd(&self) -> &str {
        &self.cmd[..]
    }

    /// Return the response values
    pub fn args(&self) -> &Vec<serde_json::Value> {
        &self.args
    }
}

/// Data to be sent back to connection in the form of either a `Notification` or a `Response`
///
/// A `Response` takes a u64 as the first argument to represent the id and a JSON object as
/// the second argument to represent the response. For example a response with an id of `1`
/// and a JSON object of `{"status":"OK"}` will be decoded by the `VIMCodec` as
/// `[1,{"status":"OK"}]` and sent as a response to the requesting socket.
///
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum PadreSend {
    Response(Response),
    Notification(Notification),
}

/// Process a TCP socket connection.
///
/// Fully sets up a new socket connection including listening for requests and sending responses.
pub fn process_connection(socket: TcpStream) {
    let addr = socket.peer_addr().unwrap();

    let (request_tx, request_rx) = VimCodec::new().framed(socket).split();

    let (connection_tx, connection_rx) = mpsc::channel(1);

    add_listener(connection_tx.clone(), addr.clone());

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

    let connection_tx_2 = connection_tx.clone();

    tokio::spawn(
        request_rx
            .and_then(move |req| respond(req))
            .for_each(move |resp| {
                tokio::spawn(
                    connection_tx_2
                        .clone()
                        .send(PadreSend::Response(resp))
                        .map(|_| {})
                        .map_err(|e| println!("Error responding: {}", e)),
                );
                Ok(())
            })
            .map_err(move |e| {
                match e.kind() {
                    // Remove socket from notifier if pipe broken, otherwise report error
                    std::io::ErrorKind::ConnectionReset => {
                        remove_listener(&addr.clone());
                    }
                    _ => {
                        eprintln!("Socket error = {:?}", e);
                    }
                }
            }),
    );

    // TODO: Get rid of this signal started stuff, requests should be queued and VIM should
    // work out when it's connected.
    tokio::spawn(
        connection_tx
            .clone()
            .send(PadreSend::Notification(Notification::new(
                "padre#debugger#SignalPADREStarted".to_string(),
                vec![],
            )))
            .map(|_| ())
            .map_err(|e| eprintln!("Notifier can't send to socket: {}", e)),
    );
}

/// Process a PadreRequest.
///
/// Forwards the request to the appropriate place to handle it and responds appropriately.
fn respond(request: PadreRequest) -> Box<dyn Future<Item = Response, Error = io::Error> + Send> {
    let json_response = match request.cmd() {
        RequestCmd::RequestPadreCmd(cmd) => match cmd {
            PadreCmd::Ping => ping(),
            PadreCmd::Pings => pings(),
            PadreCmd::SetConfig => Ok(serde_json::json!({"status":"unimplemented"})),
        },
        RequestCmd::RequestDebuggerCmd(cmd) => Ok(serde_json::json!({"status":"unimplemented"})),
    };

    let f = future::lazy(move || match json_response {
        Ok(args) => Ok(Response::new(request.id(), args)),
        Err(_) => {
            unreachable!();
        }
    });

    Box::new(f)
}

fn ping() -> Result<serde_json::Value, io::Error> {
    Ok(serde_json::json!({"status":"OK","ping":"pong"}))
}

fn pings() -> Result<serde_json::Value, io::Error> {
    log_msg(LogLevel::INFO, "pong".to_string());

    Ok(serde_json::json!({"status":"OK"}))
}
