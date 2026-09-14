#![forbid(unsafe_code)]

//! Streams that arrive as IEC 60870-5-104 application service data units.
//! One ASDU is one Stream: the APCI around it carries the sequence numbers
//! and is the transport's, the ASDU inside — type, cause, addresses,
//! objects — is a contract's.
//!
//! IEC 104 is the grid's telecontrol protocol: substations report and take
//! commands from control centres, over TCP on port 2404. The controlled
//! station listens; the controlling station connects, says STARTDT, and
//! information frames flow both ways, each acknowledged by sequence number.
//! A Receive Location listens as the controlled station and hands each I
//! frame's ASDU up; a Send Location connects as the controlling station and
//! sends one. Which is which is configuration, and both are here.
//!
//! What is here is the APCI, the start and test commands, and I frames
//! acknowledged one at a time — the window `k` of one, which is what a
//! Location that hands every ASDU to a Journey needs. The timeouts `t1` to
//! `t3` and a wider window are the next layer; TLS, IEC 62351, is the
//! transport capability's (ADR-0033).
//!
//! The origin URI carries what the APCI knew: `iec104://peer?send=5`.

pub mod apci;

use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

pub use apci::{Apdu, MAX_ASDU};
use transport::error::{Result, classify, protocol_error};
use transport::listening::{Accepting, Listening};
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};
use transport::socket;
use transport::{Arrived, Directions, Transport};

/// The controlled station's side of one connection.
pub struct Station {
    stream: TcpStream,
    peer: SocketAddr,
    received: u16,
    started: bool,
}

impl Station {
    /// The next ASDU the controlling station sends, acknowledged, or `None`
    /// when it said STOPDT or closed. STARTDT and TESTFR are answered on
    /// the way; an I frame before STARTDT is a protocol error.
    ///
    /// # Errors
    /// Where the connection broke, nothing arrived before the timeout, or
    /// the peer broke the protocol.
    pub fn next_asdu(&mut self) -> Result<Option<Arrived>> {
        loop {
            match apci::read(&mut self.stream)? {
                Some(Apdu::U {
                    command: apci::STARTDT_ACT,
                }) => {
                    self.started = true;
                    self.send(&Apdu::U {
                        command: apci::STARTDT_CON,
                    })?;
                }
                Some(Apdu::U {
                    command: apci::STOPDT_ACT,
                }) => {
                    self.started = false;
                    self.send(&Apdu::U {
                        command: apci::STOPDT_CON,
                    })?;
                    return Ok(None);
                }
                Some(Apdu::U {
                    command: apci::TESTFR_ACT,
                }) => self.send(&Apdu::U {
                    command: apci::TESTFR_CON,
                })?,
                Some(Apdu::I { send, asdu, .. }) => {
                    if !self.started {
                        return Err(protocol_error("an I frame before STARTDT"));
                    }
                    if send != self.received {
                        return Err(protocol_error(format!(
                            "send sequence {send} where {} was expected",
                            self.received
                        )));
                    }
                    self.received = (self.received + 1) & 0x7fff;
                    self.send(&Apdu::S {
                        receive: self.received,
                    })?;
                    let origin = format!("iec104://{}?send={send}", self.peer);
                    return Ok(Some(Arrived::new(origin, asdu)));
                }
                Some(Apdu::U { .. } | Apdu::S { .. }) => {}
                None => return Ok(None),
            }
        }
    }

    fn send(&mut self, apdu: &Apdu) -> Result<()> {
        self.stream
            .write_all(&apci::encode(apdu)?)
            .map_err(|e| classify("writing an APDU", &e))?;
        self.stream
            .flush()
            .map_err(|e| classify("flushing an APDU", &e))
    }
}

/// The controlling station's side of one connection, data transfer started.
pub struct Controller {
    stream: TcpStream,
    sent: u16,
}

impl Controller {
    /// Send `asdu` as one I frame and wait for it to be acknowledged.
    ///
    /// # Errors
    /// An ASDU over [`MAX_ASDU`], a peer that went away, or an
    /// acknowledgement that does not cover it.
    pub fn send_asdu(&mut self, asdu: &[u8]) -> Result<()> {
        let send = self.sent;
        self.write(&Apdu::I {
            send,
            receive: 0,
            asdu: asdu.to_vec(),
        })?;
        self.sent = (self.sent + 1) & 0x7fff;
        loop {
            match apci::read(&mut self.stream)? {
                Some(Apdu::S { receive } | Apdu::I { receive, .. }) if receive == self.sent => {
                    return Ok(());
                }
                Some(Apdu::S { receive } | Apdu::I { receive, .. }) => {
                    return Err(protocol_error(format!(
                        "acknowledged up to {receive}, not {}",
                        self.sent
                    )));
                }
                Some(Apdu::U {
                    command: apci::TESTFR_ACT,
                }) => self.write(&Apdu::U {
                    command: apci::TESTFR_CON,
                })?,
                Some(Apdu::U { .. }) => {}
                None => return Err(protocol_error("the station closed before acknowledging")),
            }
        }
    }

    /// Say STOPDT and close.
    ///
    /// # Errors
    /// Where the station went away.
    pub fn stop(mut self) -> Result<()> {
        self.write(&Apdu::U {
            command: apci::STOPDT_ACT,
        })
    }

    fn write(&mut self, apdu: &Apdu) -> Result<()> {
        self.stream
            .write_all(&apci::encode(apdu)?)
            .map_err(|e| classify("writing an APDU", &e))?;
        self.stream
            .flush()
            .map_err(|e| classify("flushing an APDU", &e))
    }
}

#[derive(Clone)]
pub struct Iec104Transport {
    bind: String,
    timeout: Option<Duration>,
}

impl Iec104Transport {
    /// Listen or connect at `bind`; `0.0.0.0:2404` is the standard port.
    #[must_use]
    pub fn new(bind: impl Into<String>) -> Self {
        Self {
            bind: bind.into(),
            timeout: None,
        }
    }

    /// Give up on a peer that stops mid-APDU.
    #[must_use]
    pub const fn timing_out_after(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Bind and report the address actually assigned.
    ///
    /// # Errors
    /// Where the address is taken, malformed, or not permitted.
    pub fn bind(&self) -> Result<(TcpListener, String)> {
        socket::bind_tcp(&self.bind)
    }

    /// Accept one controlling station on an already-bound listener.
    ///
    /// # Errors
    /// Where the connection could not be accepted.
    pub fn accept_one(&self, listener: &TcpListener) -> Result<Station> {
        let (stream, peer) = socket::accept_tcp(listener, self.timeout)?;
        Ok(Station {
            stream,
            peer,
            received: 0,
            started: false,
        })
    }

    /// Connect to the controlled station at `target` and start data
    /// transfer.
    ///
    /// # Errors
    /// Where the station refused, could not be reached, or did not confirm
    /// STARTDT.
    pub fn connect(&self, target: &str) -> Result<Controller> {
        let stream = socket::connect_tcp(target, self.timeout)?;
        let mut controller = Controller { stream, sent: 0 };
        controller.write(&Apdu::U {
            command: apci::STARTDT_ACT,
        })?;
        match apci::read(&mut controller.stream)? {
            Some(Apdu::U {
                command: apci::STARTDT_CON,
            }) => Ok(controller),
            _ => Err(protocol_error("the station did not confirm STARTDT")),
        }
    }
}

impl Transport for Iec104Transport {
    fn name(&self) -> &'static str {
        "iec-60870-5-104"
    }

    fn directions(&self) -> Directions {
        Directions::BOTH
    }

    /// One controlling station's ASDUs until it says STOPDT or closes.
    fn receive(&self) -> Result<Vec<Arrived>> {
        let (listener, _) = self.bind()?;
        let mut station = self.accept_one(&listener)?;
        let mut arrived = Vec::new();
        while let Some(asdu) = station.next_asdu()? {
            arrived.push(asdu);
        }
        Ok(arrived)
    }

    fn send(&self, target: &str, bytes: &[u8]) -> Result<()> {
        let mut controller = self.connect(target)?;
        controller.send_asdu(bytes)?;
        controller.stop()
    }
}

impl Iec104Transport {
    /// Both ends on this machine: an ephemeral local port, the loopback
    /// timeout on either station.
    #[must_use]
    pub fn loopback() -> Self {
        Self::new("127.0.0.1:0").timing_out_after(LOOPBACK_TIMEOUT)
    }
}

impl Accepting for Iec104Transport {
    fn take_one(&self, listener: &TcpListener) -> Result<Arrived> {
        let mut station = self.accept_one(listener)?;
        let mut origin = String::from("iec104://");
        let mut bytes = Vec::new();
        while let Some(arrived) = station.next_asdu()? {
            origin = arrived.origin_uri;
            bytes.extend_from_slice(&arrived.bytes);
        }
        Ok(Arrived::new(origin, bytes))
    }
}

/// A Stream longer than one ASDU travels as I frames in sequence on one
/// connection, each acknowledged before the next goes, STOPDT closing.
impl Loopback for Iec104Transport {
    fn far_end(&self) -> Result<Box<dyn FarEnd>> {
        let (listener, address) = self.bind()?;
        Ok(Box::new(Listening::new(self.clone(), listener, address)))
    }

    fn send_to(&self, address: &str, payload: &[u8]) -> Result<()> {
        let mut controller = self.clone().connect(address)?;
        for asdu in payload.chunks(MAX_ASDU) {
            controller.send_asdu(asdu)?;
        }
        controller.stop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transport::payload::edge_payloads;

    #[test]
    fn a_loopback_round_carries_a_stream_as_asdus() {
        let loopback = Iec104Transport::loopback();
        let arrived = loopback.round(b"asdu").expect("round");
        assert_eq!(arrived.bytes, b"asdu");
        assert!(arrived.origin_uri.starts_with("iec104://127.0.0.1:"));
        assert!(arrived.origin_uri.ends_with("?send=0"));
        let long = vec![0x2a; 3000];
        let arrived = loopback.round(&long).expect("thirteen I frames");
        assert_eq!(arrived.bytes, long);
        assert!(arrived.origin_uri.ends_with("?send=12"));
        assert!(loopback.ceiling().is_none());
        assert!(loopback.refuses(&long).is_none());
    }

    #[test]
    fn the_loopback_returns_the_edges_whole() {
        let loopback = Iec104Transport::loopback();
        for (name, bytes) in edge_payloads() {
            let arrived = loopback
                .round(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(arrived.bytes, bytes, "{name}");
        }
    }

    #[test]
    fn asdus_flow_in_sequence_and_are_acknowledged() {
        let station = Iec104Transport::new("127.0.0.1:0").timing_out_after(Duration::from_secs(2));
        let (listener, address) = station.bind().expect("binding");
        let controller = std::thread::spawn(move || {
            let mut controller = Iec104Transport::new("127.0.0.1:0")
                .timing_out_after(Duration::from_secs(2))
                .connect(&address)?;
            controller.send_asdu(&[0x2d, 0x01, 0x06, 0x00, 0x01, 0x00, 0x10, 0x27, 0x00, 0x01])?;
            controller.send_asdu(&[0x64, 0x01, 0x06, 0x00, 0x01, 0x00, 0, 0, 0, 0x14])?;
            controller.stop()
        });
        let mut station = station.accept_one(&listener).expect("accepting");
        let first = station.next_asdu().expect("first").expect("an ASDU");
        assert_eq!(first.bytes[0], 0x2d);
        assert!(first.origin_uri.ends_with("?send=0"));
        let second = station.next_asdu().expect("second").expect("an ASDU");
        assert_eq!(second.bytes[0], 0x64);
        assert!(second.origin_uri.ends_with("?send=1"));
        assert!(station.next_asdu().expect("stopped").is_none());
        controller.join().expect("thread").expect("controlling");
    }

    #[test]
    fn the_transport_trait_receives_a_session_and_sends_one_asdu() {
        let station = Iec104Transport::new("127.0.0.1:0").timing_out_after(Duration::from_secs(2));
        let (listener, address) = station.bind().expect("binding");
        let sender = std::thread::spawn(move || {
            Iec104Transport::new("127.0.0.1:0")
                .timing_out_after(Duration::from_secs(2))
                .send(&address, b"asdu")
        });
        let mut station = station.accept_one(&listener).expect("accepting");
        let mut arrived = Vec::new();
        while let Some(asdu) = station.next_asdu().expect("receiving") {
            arrived.push(asdu);
        }
        sender.join().expect("thread").expect("sending");
        assert_eq!(arrived.len(), 1);
        assert_eq!(arrived[0].bytes, b"asdu");
        assert!(Iec104Transport::new("127.0.0.1:0").claims().is_none());
    }

    #[test]
    fn an_i_frame_before_startdt_and_a_wrong_sequence_are_refused() {
        let station = Iec104Transport::new("127.0.0.1:0").timing_out_after(Duration::from_secs(2));
        let (listener, address) = station.bind().expect("binding");
        let rogue = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connecting");
            let early = apci::encode(&Apdu::I {
                send: 0,
                receive: 0,
                asdu: vec![1],
            })
            .expect("encode");
            stream.write_all(&early).expect("writing");
        });
        let mut station = station.accept_one(&listener).expect("accepting");
        let error = station.next_asdu().expect_err("early");
        assert!(!error.retryable);
        rogue.join().expect("thread");
        assert!(
            apci::encode(&Apdu::I {
                send: 0,
                receive: 0,
                asdu: vec![0; MAX_ASDU + 1]
            })
            .is_err()
        );
    }
}
