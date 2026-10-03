//! The controlled station's side of one IEC 104 connection: STARTDT, STOPDT
//! and TESTFR answered on the way, each I frame's ASDU acknowledged by an S
//! frame once its receive cycle has ended.

use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use transport::answer::Answer;
use transport::error::{Result, classify, protocol_error};
use transport::serving::{Open, Turn};
use transport::{Acknowledgement, Arrived, Verdict};

use crate::apci::{self, Apdu};

/// The cause of transmission's low six bits (IEC 60870-5-101 section
/// 7.2.3): activation, and its confirmation.
const ACTIVATION: u8 = 6;
const ACTIVATION_CONFIRMATION: u8 = 7;
/// Deactivation, and its confirmation.
const DEACTIVATION: u8 = 8;
const DEACTIVATION_CONFIRMATION: u8 = 9;
/// The cause octet's P/N bit: the confirmation is negative.
pub const NEGATIVE: u8 = 0x40;
/// The cause octet's test bit, kept as the command set it.
const TEST: u8 = 0x80;
/// Where the cause octet stands in an ASDU: after the type and the
/// variable structure qualifier.
pub const CAUSE: usize = 2;

/// One ASDU as it arrived, not yet acknowledged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asdu {
    /// `iec104://peer?send=5`.
    pub origin_uri: String,
    pub bytes: Vec<u8>,
    /// The receive sequence number an S frame acknowledging it carries.
    receive: u16,
}

/// The controlled station's side of one connection.
pub struct Station {
    stream: TcpStream,
    peer: SocketAddr,
    received: u16,
    /// The send sequence number of the next I frame this station sends: a
    /// negative confirmation, the one I frame it sends, from the verdict.
    sent: Arc<AtomicU16>,
    started: bool,
}

impl Station {
    /// A connection a controlling station opened from `peer`.
    #[must_use]
    pub fn new(stream: TcpStream, peer: SocketAddr) -> Self {
        Self {
            stream,
            peer,
            received: 0,
            sent: Arc::new(AtomicU16::new(0)),
            started: false,
        }
    }

    /// The next ASDU the controlling station sends, not yet acknowledged,
    /// or `None` when it said STOPDT or closed. STARTDT and TESTFR are
    /// answered on the way; an I frame before STARTDT is a protocol error.
    ///
    /// # Errors
    /// Where the connection broke, nothing arrived before the timeout, or
    /// the peer broke the protocol.
    pub fn next_asdu(&mut self) -> Result<Option<Asdu>> {
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
                    return Ok(Some(Asdu {
                        origin_uri: format!("iec104://{}?send={send}", self.peer),
                        bytes: asdu,
                        receive: self.received,
                    }));
                }
                Some(Apdu::U { .. } | Apdu::S { .. }) => {}
                None => return Ok(None),
            }
        }
    }

    /// The next ASDU as a Stream whose verdict answers the controlling
    /// station: the S frame acknowledging it on accepted; on refused, a
    /// command (cause activation or deactivation) is confirmed negatively —
    /// mirrored with the confirmation cause and the P/N bit set (IEC
    /// 60870-5-101 section 7.2.3), in an I frame that acknowledges it — so
    /// it is not sent again, and any other ASDU, which has no negative
    /// confirmation, is acknowledged by its S frame, taken and not sent
    /// again; on failed, the connection closed unacknowledged, which is how
    /// IEC 104 has a controlling station send what was not acknowledged
    /// again. `None` when it said STOPDT or closed.
    ///
    /// # Errors
    /// As [`Station::next_asdu`], or the connection could not be held for
    /// the answer.
    pub fn next_arrival(&mut self) -> Result<Option<Arrived>> {
        let Some(asdu) = self.next_asdu()? else {
            return Ok(None);
        };
        // Let go without a verdict, the connection is shut, as on failed.
        let held = Answer::held(&self.stream)?;
        let receive = asdu.receive;
        let negative = negative_confirmation(&asdu.bytes);
        let counter = Arc::clone(&self.sent);
        let acknowledgement = Acknowledgement::deferred(move |verdict| match verdict {
            Verdict::Accepted => held.with(|stream| write(stream, &Apdu::S { receive })),
            Verdict::Refused(_) => held.with(|stream| match negative {
                Some(asdu) => {
                    let send = counter.fetch_add(1, Ordering::Relaxed) & 0x7fff;
                    write(
                        stream,
                        &Apdu::I {
                            send,
                            receive,
                            asdu,
                        },
                    )
                }
                None => write(stream, &Apdu::S { receive }),
            }),
            Verdict::Failed => held.shut(),
        });
        Ok(Some(Arrived::whole(
            asdu.origin_uri,
            asdu.bytes,
            acknowledgement,
        )))
    }

    /// One turn for a kept listener: the next arrival, or the controlling
    /// station gone.
    ///
    /// # Errors
    /// As [`Station::next_arrival`].
    pub fn turn(&mut self) -> Result<Turn<Arrived>> {
        Ok(self.next_arrival()?.map_or(Turn::Closed, Turn::Taken))
    }

    /// Acknowledge `asdu` with its S frame.
    ///
    /// # Errors
    /// Where the controlling station went away.
    pub fn acknowledge(&mut self, asdu: &Asdu) -> Result<()> {
        self.send(&Apdu::S {
            receive: asdu.receive,
        })
    }

    fn send(&mut self, apdu: &Apdu) -> Result<()> {
        write(&mut self.stream, apdu)
    }
}

impl Open for Station {
    fn socket(&self) -> &TcpStream {
        &self.stream
    }
}

/// The negative confirmation of `asdu` where it is a command — its cause
/// activation or deactivation — the ASDU mirrored with the confirmation
/// cause and the P/N bit set; `None` for an ASDU that has none.
fn negative_confirmation(asdu: &[u8]) -> Option<Vec<u8>> {
    let cause = *asdu.get(CAUSE)?;
    let confirmation = match cause & 0x3f {
        ACTIVATION => ACTIVATION_CONFIRMATION,
        DEACTIVATION => DEACTIVATION_CONFIRMATION,
        _ => return None,
    };
    let mut mirrored = asdu.to_vec();
    mirrored[CAUSE] = (cause & TEST) | NEGATIVE | confirmation;
    Some(mirrored)
}

fn write(stream: &mut TcpStream, apdu: &Apdu) -> Result<()> {
    stream
        .write_all(&apci::encode(apdu)?)
        .map_err(|e| classify("writing an APDU", &e))?;
    stream.flush().map_err(|e| classify("flushing an APDU", &e))
}
