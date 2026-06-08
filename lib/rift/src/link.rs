use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;

use crate::client::Client;
use crate::codec::Wire;

pub trait Transport {
    fn send(&mut self, packet: &[u8]);
    fn poll(&mut self, sink: &mut dyn FnMut(&[u8]));
    fn status(&self) -> LinkStatus;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkStatus {
    Connecting,
    Open,
    Closed,
}

pub struct Link {
    pub client: Client,
    transport: Box<dyn Transport>,
}

impl Link {
    pub fn tcp(address: &str, token: &str) -> std::io::Result<Self> {
        Ok(Self::new(Box::new(TcpTransport::connect(address, token)?)))
    }

    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            client: Client::new(),
            transport,
        }
    }

    pub fn poll(&mut self) {
        let Self { client, transport } = self;
        transport.poll(&mut |packet| client.receive(packet));
    }

    pub fn send<E: Wire + 'static>(&mut self, event: &E) {
        let bytes = self.client.send(event);
        self.transport.send(&bytes);
    }

    pub fn status(&self) -> LinkStatus {
        self.transport.status()
    }
}

pub(crate) const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

pub(crate) fn frame(payload: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
}

// `Err` flags a frame larger than the protocol allows (a hostile or corrupt peer).
pub(crate) fn unframe_one(buffer: &mut Vec<u8>) -> Result<Option<Vec<u8>>, ()> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let length = u32::from_le_bytes(buffer[..4].try_into().map_err(|_| ())?) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(());
    }
    if buffer.len() - 4 < length {
        return Ok(None);
    }
    let payload = buffer[4..4 + length].to_vec();
    buffer.drain(..4 + length);
    Ok(Some(payload))
}

pub(crate) fn pump(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> bool {
    let mut scratch = [0u8; 8192];
    loop {
        match stream.read(&mut scratch) {
            Ok(0) => return false,
            Ok(read) => buffer.extend_from_slice(&scratch[..read]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => return true,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
}

struct TcpTransport {
    stream: TcpStream,
    buffer: Vec<u8>,
    open: bool,
}

impl TcpTransport {
    fn connect(address: &str, token: &str) -> std::io::Result<Self> {
        let mut stream = TcpStream::connect(address)?;
        stream.set_nodelay(true).ok();
        let mut handshake = Vec::new();
        frame(token.as_bytes(), &mut handshake);
        stream.write_all(&handshake)?;
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            buffer: Vec::new(),
            open: true,
        })
    }
}

impl Transport for TcpTransport {
    fn send(&mut self, packet: &[u8]) {
        let mut framed = Vec::with_capacity(4 + packet.len());
        frame(packet, &mut framed);
        let _ = self.stream.write_all(&framed);
    }

    fn poll(&mut self, sink: &mut dyn FnMut(&[u8])) {
        if !pump(&mut self.stream, &mut self.buffer) {
            self.open = false;
        }
        loop {
            match unframe_one(&mut self.buffer) {
                Ok(Some(packet)) => sink(&packet),
                Ok(None) => break,
                Err(()) => {
                    self.open = false;
                    break;
                }
            }
        }
    }

    fn status(&self) -> LinkStatus {
        if self.open {
            LinkStatus::Open
        } else {
            LinkStatus::Closed
        }
    }
}
