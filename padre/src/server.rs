//! handle server connections

use std::collections::HashMap;
use std::io;

use crate::request::PadreRequest;

use bytes::{BufMut, Bytes, BytesMut};
use futures::sync::mpsc::{self, UnboundedReceiver};
use tokio::codec::{Decoder, Encoder, Framed};
use tokio::io::ReadHalf;
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::prelude::stream::{SplitSink, SplitStream};

pub fn process_connection(socket: TcpStream) {
    let (tx, rx) = PadreCodec{}.framed(socket).split();

    tokio::spawn(
        rx.for_each(|request| {
            println!("Request: {:?}", request);
            Ok(())
        }).map_err(|e| {
            println!("connection error = {:?}", e);
        })
    );
}

#[derive(Debug)]
pub struct PadreConnection {
    reader: SplitStream<Framed<TcpStream, PadreCodec>>,
    writer_rx: UnboundedReceiver<Bytes>,
    rd: BytesMut,
}

impl PadreConnection {
    pub fn new(socket: TcpStream) -> Self {
        let (writer_tx, writer_rx) = mpsc::unbounded();

        let (writer, reader) = PadreCodec{}.framed(socket).split();

        PadreConnection {
            reader,
            writer_rx,
            rd: BytesMut::new(),
        }
    }
//
//    fn fill_read_buf(&mut self) -> Poll<(), io::Error> {
//        loop {
//            self.rd.reserve(1024);
//
//            let n = try_ready!(self.reader.read_buf(&mut self.rd));
//
//            if n == 0 {
//                return Ok(Async::Ready(()));
//            }
//        }
//    }
}

impl Stream for PadreConnection {
    type Item = PadreRequest;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
//        let sock_closed = self.fill_read_buf()?.is_ready();
//
//        if sock_closed {
//            Ok(Async::Ready(None))
//        } else {
            Ok(Async::NotReady)
//        }
    }
}

#[derive(Debug)]
struct PadreCodec {}

impl Decoder for PadreCodec {
    type Item = PadreRequest;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        println!("Decoding: {:?}", src);
        let mut v: serde_json::Value = serde_json::from_slice(&src).unwrap();
        let id: u32 = serde_json::from_value(v[0].take()).unwrap();
        let cmd: String = serde_json::from_value(v[1]["cmd"].take()).unwrap();
        let padre_request: PadreRequest = PadreRequest::new(id, cmd);

        Ok(Some(padre_request))
    }
}

impl Encoder for PadreCodec {
    type Item = PadreRequest;
    type Error = io::Error;

    fn encode(&mut self, padre_cmd: PadreRequest, buf: &mut BytesMut) -> Result<(), io::Error> {
        println!("Encoding");
        let id = padre_cmd.id();

        let mut args: HashMap<String, String> = HashMap::new();
        args.insert("cmd".to_string(), padre_cmd.cmd().clone().to_string());

        let response = serde_json::to_string(&(id, args)).unwrap();

        buf.reserve(response.len());
        buf.put(&response[..]);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::request::PadreRequest;
    use bytes::{BufMut, Bytes, BytesMut};
    use tokio::codec::{Decoder, Encoder};

    #[test]
    fn check_single_json_decoding() {
        let mut codec = super::PadreCodec{};
        let mut buf = BytesMut::new();
        buf.reserve(19);
        buf.put("[123,{\"cmd\":\"run\"}]");

        let padre_request = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(PadreRequest::new(123, "run".to_string()), padre_request);
    }

    #[test]
    fn check_json_encoding() {
        let mut codec = super::PadreCodec{};
        let padre_cmd = PadreRequest::new(123, "run".to_string());
        let mut buf = BytesMut::new();
        codec.encode(padre_cmd, &mut buf);

        let mut expected = BytesMut::new();
        expected.reserve(19);
        expected.put("[123,{\"cmd\":\"run\"}]");

        assert_eq!(expected, buf);
    }
}
