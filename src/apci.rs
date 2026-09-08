//! IEC 60870-5-104 section 5: the application protocol control information
//! — a start byte, a length, four control-field bytes — and the three frame
//! formats the control field encodes: I for data with both sequence
//! numbers, S for an acknowledgement, U for the start, stop and test
//! commands.

use std::io::Read;

use transport::error::{Result, classify, protocol_error};

/// The start byte every APDU opens with.
pub const START: u8 = 0x68;
/// The most an APDU may be after the start and length bytes.
pub const MAX_APDU: usize = 253;
/// The most ASDU one I frame carries: the APDU less the control field.
pub const MAX_ASDU: usize = MAX_APDU - 4;

/// The U-format commands, as the first control byte writes them.
pub const STARTDT_ACT: u8 = 0x07;
pub const STARTDT_CON: u8 = 0x0b;
pub const STOPDT_ACT: u8 = 0x13;
pub const STOPDT_CON: u8 = 0x23;
pub const TESTFR_ACT: u8 = 0x43;
pub const TESTFR_CON: u8 = 0x83;

/// One APDU, by format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Apdu {
    /// Information: the send and receive sequence numbers and an ASDU.
    I {
        send: u16,
        receive: u16,
        asdu: Vec<u8>,
    },
    /// Supervisory: a receive sequence number, acknowledging up to it.
    S { receive: u16 },
    /// Unnumbered: one of the six commands.
    U { command: u8 },
}

/// Encode `apdu`.
///
/// # Errors
/// An ASDU over [`MAX_ASDU`].
pub fn encode(apdu: &Apdu) -> Result<Vec<u8>> {
    let mut out = vec![START, 4];
    match apdu {
        Apdu::I {
            send,
            receive,
            asdu,
        } => {
            if asdu.len() > MAX_ASDU {
                return Err(protocol_error("an ASDU over what one APDU carries"));
            }
            out[1] = u8::try_from(4 + asdu.len()).unwrap_or(u8::MAX);
            out.extend_from_slice(&(send << 1).to_le_bytes());
            out.extend_from_slice(&(receive << 1).to_le_bytes());
            out.extend_from_slice(asdu);
        }
        Apdu::S { receive } => {
            out.extend_from_slice(&[0x01, 0x00]);
            out.extend_from_slice(&(receive << 1).to_le_bytes());
        }
        Apdu::U { command } => {
            out.extend_from_slice(&[*command, 0x00, 0x00, 0x00]);
        }
    }
    Ok(out)
}

/// Read one APDU, or `None` when the peer closed between APDUs.
///
/// # Errors
/// A connection that closes mid-APDU, no start byte, a length under four
/// or over [`MAX_APDU`], or a U command that does not exist.
pub fn read(reader: &mut impl Read) -> Result<Option<Apdu>> {
    let mut head = [0u8; 2];
    let first = reader
        .read(&mut head[..1])
        .map_err(|e| classify("reading the start byte", &e))?;
    if first == 0 {
        return Ok(None);
    }
    if head[0] != START {
        return Err(protocol_error("a byte that is not the APDU start"));
    }
    reader
        .read_exact(&mut head[1..])
        .map_err(|e| classify("reading the APDU length", &e))?;
    let length = usize::from(head[1]);
    if !(4..=MAX_APDU).contains(&length) {
        return Err(protocol_error("an APDU length outside four to 253"));
    }
    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|e| classify("reading the APDU", &e))?;
    let control = [body[0], body[1], body[2], body[3]];
    let receive = u16::from_le_bytes([control[2], control[3]]) >> 1;
    let apdu = match control[0] & 0x03 {
        0x01 => Apdu::S { receive },
        0x03 => {
            if ![
                STARTDT_ACT,
                STARTDT_CON,
                STOPDT_ACT,
                STOPDT_CON,
                TESTFR_ACT,
                TESTFR_CON,
            ]
            .contains(&control[0])
            {
                return Err(protocol_error("a U command that does not exist"));
            }
            Apdu::U {
                command: control[0],
            }
        }
        _ => Apdu::I {
            send: u16::from_le_bytes([control[0], control[1]]) >> 1,
            receive,
            asdu: body[4..].to_vec(),
        },
    };
    Ok(Some(apdu))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_round_trips() {
        for apdu in [
            Apdu::I {
                send: 5,
                receive: 3,
                asdu: vec![0x0d, 0x01, 0x03, 0x00, 0x01, 0x00, 0x0a, 0x00, 0x00],
            },
            Apdu::I {
                send: 32767,
                receive: 0,
                asdu: Vec::new(),
            },
            Apdu::S { receive: 7 },
            Apdu::U {
                command: STARTDT_ACT,
            },
            Apdu::U {
                command: TESTFR_CON,
            },
        ] {
            let bytes = encode(&apdu).expect("encode");
            assert_eq!(bytes[0], START);
            assert_eq!(usize::from(bytes[1]), bytes.len() - 2);
            let back = read(&mut bytes.as_slice()).expect("read").expect("one");
            assert_eq!(back, apdu);
        }
        assert_eq!(
            encode(&Apdu::S { receive: 1 }).expect("s"),
            [0x68, 4, 1, 0, 2, 0]
        );
    }

    #[test]
    fn what_is_not_an_apdu_is_refused() {
        assert!(read(&mut &[][..]).expect("closed").is_none());
        assert!(read(&mut &[0x69, 4, 0, 0, 0, 0][..]).is_err(), "start");
        assert!(read(&mut &[0x68, 3, 0, 0, 0][..]).is_err(), "short");
        assert!(
            read(&mut &[0x68, 4, 0x33, 0, 0, 0][..]).is_err(),
            "U command"
        );
        assert!(read(&mut &[0x68, 6, 0, 0, 0][..]).is_err(), "cut off");
        assert!(
            encode(&Apdu::I {
                send: 0,
                receive: 0,
                asdu: vec![0; MAX_ASDU + 1]
            })
            .is_err()
        );
    }
}
