use bytes::{BigEndian, BufMut, BytesMut};

use std::io;

use tokio_io::codec::{Decoder, Encoder};

pub struct QuicCodec {}

impl Decoder for QuicCodec {
    type Item = Packet;
    type Error = io::Error;
    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, io::Error> {
        let first = buf[0];
        let (header, number, offset) = if first & 128 == 128 {

            let h = Header::Long {
                ptype: LongType::from_byte(first ^ 128),
                conn_id: bytes_to_u64(&buf[1..9]),
                version: bytes_to_u32(&buf[9..13]),
            };
            let number = bytes_to_u32(&buf[13..17]);
            (h, number, 17)

        } else {

            let number_size = NumberSize::from_byte(first & 7);
            let conn_id = if first & 0x40 == 0x40 {
                Some(bytes_to_u64(&buf[1..9]))
            } else {
                None
            };
            let h = Header::Short {
                number_size,
                conn_id,
                key_phase: first & 0x20 == 0x20,
            };

            let offset = if conn_id.is_some() { 5 } else { 1 };
            let size = h.number_size();
            let number = if size == 1 {
                buf[offset] as u32
            } else if size == 2 {
                (buf[offset] as u32) << 8 | (buf[offset + 1] as u32)
            } else {
                bytes_to_u32(&buf[offset..offset + 4])
            };
            (h, number, offset + size)

        };
        Ok(Some(Packet {
            header,
            number,
            payload: Vec::new(),
        }))
    }
}

impl Encoder for QuicCodec {
    type Item = Packet;
    type Error = io::Error;
    fn encode(&mut self, msg: Self::Item, dst: &mut BytesMut) -> Result<(), io::Error> {
        match msg.header {
            Header::Long { ptype, conn_id, version } => {
                dst.put(128 | ptype.to_byte());
            },
            Header::Short { number_size, conn_id, key_phase } => {

            },
        }
        Ok(())
    }
}

pub struct Packet {
    pub header: Header,
    pub number: u32,
    pub payload: Vec<Frame>,
}

pub enum Header {
    Short {
        number_size: NumberSize,
        conn_id: Option<u64>,
        key_phase: bool,
    },
    Long {
        ptype: LongType,
        conn_id: u64,
        version: u32,
    },
}

impl Header {
    fn number_size(&self) -> usize {
        match *self {
            Header::Short { ref number_size, .. } => number_size.number_size(),
            Header::Long { .. } => 4,
        }
    }
}

pub enum LongType {
    Initial = 0x7f,
    Retry = 0x7e,
    Handshake = 0x7d,
    Protected = 0x7c,
}

impl LongType {
    fn to_byte(&self) -> u8 {
        use self::LongType::*;
        match *self {
            Initial => 0x7f,
            Retry => 0x7e,
            Handshake => 0x7d,
            Protected => 0x7c,
        }
    }
    fn from_byte(v: u8) -> Self {
        use self::LongType::*;
        match v {
            0x7f => Initial,
            0x7e => Retry,
            0x7d => Handshake,
            0x7c => Protected,
            _ => panic!("invalid long packet type {}", v),
        }
    }
}

pub enum NumberSize {
    One = 0x0,
    Two = 0x1,
    Four = 0x2,
}

impl NumberSize {
    fn number_size(&self) -> usize {
        use self::NumberSize::*;
        match *self {
            One => 1,
            Two => 2,
            Four => 4,
        }
    }
    fn from_byte(v: u8) -> Self {
        use self::NumberSize::*;
        match v {
            0 => One,
            1 => Two,
            2 => Four,
            _ => panic!("invalid short packet type {}", v),
        }
    }
}

pub enum Frame {
    Stream(StreamFrame),
}

pub struct StreamFrame {
    pub id: u64,
    pub offset: Option<u64>,
    pub length: Option<u64>,
    pub data: Vec<u8>,
}

pub enum FrameType {
    Padding = 0x0,
    ResetStream = 0x1,
    ConnectionClose = 0x2,
    ApplicationClose = 0x3,
    MaxData = 0x4,
    MaxStreamData = 0x5,
    MaxStreamId = 0x6,
    Ping = 0x7,
    Blocked = 0x8,
    StreamBlocked = 0x9,
    StreamIdBlocked = 0xa,
    NewConnectionId = 0xb,
    StopSending = 0xc,
    Ack = 0xd,
    PathChallenge = 0xe,
    PathResponse = 0xf,
    Stream = 0x10,
    StreamFin = 0x11,
    StreamLen = 0x12,
    StreamLenFin = 0x13,
    StreamOff = 0x14,
    StreamOffFin = 0x15,
    StreamOffLen = 0x16,
    StreamOffLenFin = 0x17,
}

struct VarLen {
    val: u64,
}

impl VarLen {
    fn new(val: u64) -> VarLen {
        VarLen { val }
    }
}

impl BufLen for VarLen {
    fn buf_len(&self) -> usize {
        match self.val {
            v if v <= 63 => 1,
            v if v <= 16_383 => 2,
            v if v <= 1_073_741_823 => 4,
            v if v <= 4_611_686_018_427_387_903 => 8,
            v => panic!("too large for variable-length encoding: {}", v),
        }
    }
}

impl Codec for VarLen {
    fn encode<T: BufMut>(&self, buf: &mut T) {
        match self.buf_len() {
            1 => buf.put_u8(self.val as u8),
            2 => buf.put_u16::<BigEndian>(self.val as u16 | 16384),
            4 => buf.put_u32::<BigEndian>(self.val as u32 | 2_147_483_648),
            8 => buf.put_u64::<BigEndian>(self.val | 13_835_058_055_282_163_712),
            _ => panic!("impossible variable-length encoding"),
        }
    }
}

fn bytes_to_u64(bytes: &[u8]) -> u64 {
    debug_assert_eq!(bytes.len(), 8);
    ((bytes[0] as u64) << 56 |
        (bytes[1] as u64) << 48 |
        (bytes[2] as u64) << 40 |
        (bytes[3] as u64) << 32 |
        (bytes[4] as u64) << 24 |
        (bytes[5] as u64) << 16 |
        (bytes[6] as u64) << 8 |
        (bytes[7] as u64))
}

fn bytes_to_u32(bytes: &[u8]) -> u32 {
    debug_assert_eq!(bytes.len(), 4);
    ((bytes[0] as u32) << 24 |
        (bytes[1] as u32) << 16 |
        (bytes[2] as u32) << 8 |
        (bytes[3] as u32))
}

trait BufLen {
    fn buf_len(&self) -> usize;
}

impl<T> BufLen for Option<T> where T: BufLen {
    fn buf_len(&self) -> usize {
        match *self {
            Some(ref v) => v.buf_len(),
            None => 0,
        }
    }
}

trait Codec {
    fn encode<T: BufMut>(&self, buf: &mut T);
}

#[cfg(test)]
mod tests {
    use super::{Codec, VarLen};
    #[test]
    fn test_var_len_encoding_8() {
        let num = 151_288_809_941_952_652;
        let bytes = b"\xc2\x19\x7c\x5e\xff\x14\xe8\x8c";
        let mut buf = Vec::new();
        VarLen::new(num).encode(&mut buf);
        assert_eq!(bytes[..], *buf);
    }
    #[test]
    fn test_var_len_encoding_4() {
        let num = 494_878_333;
        let bytes = b"\x9d\x7f\x3e\x7d";
        let mut buf = Vec::new();
        VarLen::new(num).encode(&mut buf);
        assert_eq!(bytes[..], *buf);
    }
    #[test]
    fn test_var_len_encoding_2() {
        let num = 15_293;
        let bytes = b"\x7b\xbd";
        let mut buf = Vec::new();
        VarLen::new(num).encode(&mut buf);
        assert_eq!(bytes[..], *buf);
    }
    #[test]
    fn test_var_len_encoding_1_short() {
        let num = 37;
        let bytes = b"\x25";
        let mut buf = Vec::new();
        VarLen::new(num).encode(&mut buf);
        assert_eq!(bytes[..], *buf);
    }
}
