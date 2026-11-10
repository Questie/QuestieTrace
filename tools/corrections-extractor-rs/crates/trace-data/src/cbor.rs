//! Zero-copy cursor over the CBOR subset Blizzard's serializer writes: definite lengths, no
//! tags, simple values false/true/null/undefined, half/single/double floats.
//!
//! Error messages match `core/trace-data/cbor.ts` word for word: they end up in meta.json.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct CborError(pub String);

impl fmt::Display for CborError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CborError {}

pub type Result<T> = std::result::Result<T, CborError>;

fn err<T>(message: &str) -> Result<T> {
    Err(CborError(message.to_owned()))
}

/// Tables nested deeper than this are rejected, as by the Lua reference decoder.
pub const MAX_NESTING_DEPTH: u32 = 100;

/// A scalar, or the header of a container whose items follow it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Item<'a> {
    Num(f64),
    /// A text or byte string, not yet checked for UTF-8.
    Str(&'a [u8]),
    Bool(bool),
    /// Lua nil (CBOR null or undefined).
    Nil,
    Array(f64),
    Map(f64),
}

pub struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Moves back to a position read before, to walk a container again.
    pub fn set_pos(&mut self, pos: usize) {
        debug_assert!(pos <= self.bytes.len());
        self.pos = pos;
    }

    /// How many items of a container with `length` items to allocate room for: every item takes
    /// at least one byte, so a hostile length cannot reserve more than the data left.
    pub fn capacity_for(&self, length: f64) -> usize {
        let left = self.bytes.len() - self.pos;
        if length < left as f64 {
            length as usize
        } else {
            left
        }
    }

    /// The major type of the next item, without reading it.
    pub fn peek_major(&self) -> Option<u8> {
        self.bytes.get(self.pos).map(|initial| initial >> 5)
    }

    pub fn is_done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// The bytes from `start` to the cursor.
    pub fn slice_from(&self, start: usize) -> &'a [u8] {
        &self.bytes[start..self.pos]
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        match self.pos.checked_add(count).filter(|&end| end <= self.bytes.len()) {
            Some(end) => {
                let slice = &self.bytes[self.pos..end];
                self.pos = end;
                Ok(slice)
            }
            None => err("truncated CBOR data"),
        }
    }

    /// Big-endian unsigned integer of `N` bytes.
    #[inline]
    fn uint<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("take(N) returns N bytes"))
    }

    /// The argument of an item header. The Lua and TS decoders accumulate it byte by byte in
    /// doubles; up to 2^53 that is exact, so only larger 8-byte values take that path to round
    /// the same way.
    #[inline]
    fn argument(&mut self, info: u8) -> Result<f64> {
        match info {
            0..=23 => Ok(info as f64),
            24 => Ok(u8::from_be_bytes(self.uint()?) as f64),
            25 => Ok(u16::from_be_bytes(self.uint()?) as f64),
            26 => Ok(u32::from_be_bytes(self.uint()?) as f64),
            27 => {
                let bytes: [u8; 8] = self.uint()?;
                let value = u64::from_be_bytes(bytes);
                if value <= 1 << 53 {
                    Ok(value as f64)
                } else {
                    Ok(bytes.iter().fold(0.0, |value, &byte| value * 256.0 + byte as f64))
                }
            }
            31 => err("indefinite-length CBOR items are not supported"),
            _ => err("invalid CBOR additional information"),
        }
    }

    /// Reads one scalar, or the header of an array or map.
    #[inline]
    pub fn item(&mut self) -> Result<Item<'a>> {
        let Some(&initial) = self.bytes.get(self.pos) else {
            return err("truncated CBOR data");
        };
        self.pos += 1;
        let major = initial >> 5;
        let info = initial & 31;
        match major {
            0 => Ok(Item::Num(self.argument(info)?)),
            1 => Ok(Item::Num(-1.0 - self.argument(info)?)),
            2 | 3 => {
                let length = self.argument(info)?;
                // A length beyond the buffer is truncated data, whatever its size.
                let length = if length > usize::MAX as f64 { usize::MAX } else { length as usize };
                Ok(Item::Str(self.take(length)?))
            }
            4 => Ok(Item::Array(self.argument(info)?)),
            5 => Ok(Item::Map(self.argument(info)?)),
            6 => err("CBOR tags are not supported"),
            _ => self.simple_or_float(info),
        }
    }

    fn simple_or_float(&mut self, info: u8) -> Result<Item<'a>> {
        match info {
            20 => Ok(Item::Bool(false)),
            21 => Ok(Item::Bool(true)),
            22 | 23 => Ok(Item::Nil),
            24 => {
                let value = u8::from_be_bytes(self.uint()?);
                Err(CborError(format!("unsupported CBOR simple value {value}")))
            }
            25 => Ok(Item::Num(half_to_f64(u16::from_be_bytes(self.uint()?)))),
            26 => Ok(Item::Num(f32::from_be_bytes(self.uint()?) as f64)),
            27 => Ok(Item::Num(f64::from_be_bytes(self.uint()?))),
            31 => err("CBOR break values are not supported"),
            _ => Err(CborError(format!("unsupported CBOR simple value {info}"))),
        }
    }
}

fn half_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = ((bits >> 10) & 0x1f) as i32;
    let mantissa = (bits & 0x3ff) as f64;
    match exponent {
        0 => sign * mantissa * 2f64.powi(-24),
        31 if mantissa == 0.0 => sign * f64::INFINITY,
        31 => f64::NAN,
        _ => sign * (1.0 + mantissa / 1024.0) * 2f64.powi(exponent - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_decode_to_known_values() {
        assert_eq!(half_to_f64(0x3c00), 1.0);
        assert_eq!(half_to_f64(0x4100), 2.5);
        assert_eq!(half_to_f64(0xc000), -2.0);
        assert!(half_to_f64(0x8000).is_sign_negative());
    }

    #[test]
    fn negative_integers_and_floats_read_as_numbers() {
        // -3, 1.5 as a double
        let bytes = [0x22, 0xfb, 0x3f, 0xf8, 0, 0, 0, 0, 0, 0];
        let mut c = Cursor::new(&bytes);
        assert_eq!(c.item().unwrap(), Item::Num(-3.0));
        assert_eq!(c.item().unwrap(), Item::Num(1.5));
        assert!(c.is_done());
    }

    #[test]
    fn truncated_data_is_an_error_with_the_ts_message() {
        let mut c = Cursor::new(&[0x62, b'a']);
        assert_eq!(c.item(), Err(CborError("truncated CBOR data".into())));
    }

    #[test]
    fn unsupported_items_name_themselves_like_the_ts_decoder() {
        assert_eq!(Cursor::new(&[0xc0]).item(), Err(CborError("CBOR tags are not supported".into())));
        assert_eq!(
            Cursor::new(&[0x9f]).item(),
            Err(CborError("indefinite-length CBOR items are not supported".into()))
        );
        assert_eq!(Cursor::new(&[0xf8, 0x20]).item(), Err(CborError("unsupported CBOR simple value 32".into())));
        assert_eq!(Cursor::new(&[0xe0]).item(), Err(CborError("unsupported CBOR simple value 0".into())));
    }
}
