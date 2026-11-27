//! The CBOR this crate writes and reads back: pack records, the pack index and the value
//! streams.
//!
//! `trace_data::cbor::Cursor` reads the addon's traces and returns every number as a double.
//! A value stream holds differences of up to 2^54, which a double cannot carry exactly, so the
//! pack's own structures are read with [`Reader`], which keeps integers whole.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct WireError(pub String);

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WireError {}

pub type Result<T> = std::result::Result<T, WireError>;

fn err<T>(message: impl Into<String>) -> Result<T> {
    Err(WireError(message.into()))
}

const MAJOR_UNSIGNED: u8 = 0;
const MAJOR_NEGATIVE: u8 = 1;
const MAJOR_BYTES: u8 = 2;
const MAJOR_TEXT: u8 = 3;
const MAJOR_ARRAY: u8 = 4;

// ---- Writing ----

/// An item header: the major type with its argument in the shortest form.
fn write_head(out: &mut Vec<u8>, major: u8, argument: u64) {
    let major = major << 5;
    match argument {
        0..=23 => out.push(major | argument as u8),
        24..=0xff => out.extend([major | 24, argument as u8]),
        0x100..=0xffff => {
            out.push(major | 25);
            out.extend((argument as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(major | 26);
            out.extend((argument as u32).to_be_bytes());
        }
        _ => {
            out.push(major | 27);
            out.extend(argument.to_be_bytes());
        }
    }
}

pub fn write_uint(out: &mut Vec<u8>, value: u64) {
    write_head(out, MAJOR_UNSIGNED, value);
}

pub fn write_int(out: &mut Vec<u8>, value: i64) {
    if value >= 0 {
        write_head(out, MAJOR_UNSIGNED, value as u64);
    } else {
        // CBOR stores a negative integer n as -1 - n, which for an i64 is its bitwise complement.
        write_head(out, MAJOR_NEGATIVE, !(value as u64));
    }
}

pub fn write_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    write_head(out, MAJOR_BYTES, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

pub fn write_text(out: &mut Vec<u8>, text: &str) {
    write_head(out, MAJOR_TEXT, text.len() as u64);
    out.extend_from_slice(text.as_bytes());
}

pub fn write_array_head(out: &mut Vec<u8>, length: usize) {
    write_head(out, MAJOR_ARRAY, length as u64);
}

/// Writes a finite float in the shortest of CBOR's three widths that holds it exactly, which is
/// how the addon's serializer writes every float. A value restored from a pack therefore gets
/// the bytes it had in the export.
pub fn write_float_shortest(out: &mut Vec<u8>, value: f64) {
    if let Some(half) = half_bits(value) {
        out.push(0xf9);
        out.extend(half.to_be_bytes());
    } else if (value as f32) as f64 == value {
        out.push(0xfa);
        out.extend((value as f32).to_be_bytes());
    } else {
        out.push(0xfb);
        out.extend(value.to_be_bytes());
    }
}

/// How many payload bytes [`write_float_shortest`] uses for `value`: 2, 4 or 8.
pub fn shortest_float_width(value: f64) -> usize {
    if half_bits(value).is_some() {
        2
    } else if (value as f32) as f64 == value {
        4
    } else {
        8
    }
}

/// The half-precision bits of a finite `value`, when a half float holds it exactly.
fn half_bits(value: f64) -> Option<u16> {
    let single = value as f32;
    if !value.is_finite() || single as f64 != value {
        return None;
    }
    let bits = single.to_bits();
    let sign = ((bits >> 31) as u16) << 15;
    let biased_exponent = (bits >> 23) & 0xff;
    let mantissa = bits & 0x7f_ffff;
    if biased_exponent == 0 {
        // Zero is a half float; a subnormal single is far below a half float's range.
        return (mantissa == 0).then_some(sign);
    }
    let exponent = biased_exponent as i32 - 127;
    // Normal half: exponent -14..=15 and a 10-bit mantissa.
    if (-14..=15).contains(&exponent) {
        return (mantissa & 0x1fff == 0).then(|| sign | (((exponent + 15) as u16) << 10) | (mantissa >> 13) as u16);
    }
    // Subnormal half: a multiple of 2^-24 below 2^-14.
    if (-24..-14).contains(&exponent) {
        let significand = (1 << 23) | mantissa;
        let shift = (-1 - exponent) as u32;
        return (significand & ((1 << shift) - 1) == 0).then(|| sign | (significand >> shift) as u16);
    }
    None
}

// ---- Reading ----

/// Reads the items this crate writes. Only definite lengths and the four major types above.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, pos: 0 }
    }

    pub fn is_done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        match self.pos.checked_add(count).filter(|&end| end <= self.bytes.len()) {
            Some(end) => {
                let slice = &self.bytes[self.pos..end];
                self.pos = end;
                Ok(slice)
            }
            None => err("truncated pack data"),
        }
    }

    fn head(&mut self) -> Result<(u8, u64)> {
        let initial = self.take(1)?[0];
        let argument = match initial & 31 {
            info @ 0..=23 => info as u64,
            24 => self.take(1)?[0] as u64,
            25 => u16::from_be_bytes(self.take(2)?.try_into().expect("two bytes")) as u64,
            26 => u32::from_be_bytes(self.take(4)?.try_into().expect("four bytes")) as u64,
            27 => u64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes")),
            _ => return err("unsupported item in pack data"),
        };
        Ok((initial >> 5, argument))
    }

    fn expect(&mut self, major: u8, what: &str) -> Result<u64> {
        match self.head()? {
            (found, argument) if found == major => Ok(argument),
            _ => err(format!("pack data: expected {what}")),
        }
    }

    pub fn uint(&mut self) -> Result<u64> {
        self.expect(MAJOR_UNSIGNED, "an unsigned integer")
    }

    pub fn int(&mut self) -> Result<i64> {
        match self.head()? {
            (MAJOR_UNSIGNED, argument) => i64::try_from(argument).or_else(|_| err("pack data: integer out of range")),
            (MAJOR_NEGATIVE, argument) => i64::try_from(argument).map(|n| !n).or_else(|_| err("pack data: integer out of range")),
            _ => err("pack data: expected an integer"),
        }
    }

    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let length = self.expect(MAJOR_BYTES, "a byte string")?;
        self.take(usize::try_from(length).unwrap_or(usize::MAX))
    }

    pub fn text(&mut self) -> Result<&'a str> {
        let length = self.expect(MAJOR_TEXT, "a text string")?;
        let bytes = self.take(usize::try_from(length).unwrap_or(usize::MAX))?;
        std::str::from_utf8(bytes).or_else(|_| err("pack data: text is not UTF-8"))
    }

    /// The length of an array whose items follow. Every item takes at least one byte, so a
    /// length beyond the data left is rejected before anything is allocated for it.
    pub fn array(&mut self) -> Result<usize> {
        let length = self.expect(MAJOR_ARRAY, "an array")?;
        match usize::try_from(length) {
            Ok(length) if length <= self.bytes.len() - self.pos => Ok(length),
            _ => err("truncated pack data"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float_bytes(value: f64) -> Vec<u8> {
        let mut out = Vec::new();
        write_float_shortest(&mut out, value);
        out
    }

    #[test]
    fn floats_take_the_shortest_width_that_holds_them_exactly() {
        // RFC 8949 appendix A.
        assert_eq!(float_bytes(0.0), [0xf9, 0x00, 0x00]);
        assert_eq!(float_bytes(-0.0), [0xf9, 0x80, 0x00]);
        assert_eq!(float_bytes(1.0), [0xf9, 0x3c, 0x00]);
        assert_eq!(float_bytes(1.5), [0xf9, 0x3e, 0x00]);
        assert_eq!(float_bytes(65504.0), [0xf9, 0x7b, 0xff]);
        assert_eq!(float_bytes(5.960464477539063e-8), [0xf9, 0x00, 0x01]);
        assert_eq!(float_bytes(0.00006103515625), [0xf9, 0x04, 0x00]);
        assert_eq!(float_bytes(-4.0), [0xf9, 0xc4, 0x00]);
        assert_eq!(float_bytes(100000.0), [0xfa, 0x47, 0xc3, 0x50, 0x00]);
        assert_eq!(float_bytes(3.4028234663852886e38), [0xfa, 0x7f, 0x7f, 0xff, 0xff]);
        assert_eq!(float_bytes(1.1), [0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a]);
        assert_eq!(float_bytes(1.0e300), [0xfb, 0x7e, 0x37, 0xe4, 0x3c, 0x88, 0x00, 0x75, 0x9c]);
    }

    #[test]
    fn a_half_float_is_used_only_when_no_bit_is_lost() {
        // One bit below what a half float's 10-bit mantissa holds.
        assert_eq!(shortest_float_width(1.0 + 1.0 / 1024.0), 2);
        assert_eq!(shortest_float_width(1.0 + 1.0 / 2048.0), 4);
        // The smallest half subnormal, and half of it.
        assert_eq!(shortest_float_width(2f64.powi(-24)), 2);
        assert_eq!(shortest_float_width(2f64.powi(-25)), 4);
        // Just above the largest half float.
        assert_eq!(shortest_float_width(65536.0), 4);
    }

    #[test]
    fn integers_survive_at_both_ends_of_their_range() {
        let values = [
            0,
            23,
            24,
            255,
            256,
            65535,
            65536,
            u32::MAX as i64,
            u32::MAX as i64 + 1,
            i64::MAX,
            -1,
            -24,
            -25,
            -256,
            -257,
            i64::MIN,
        ];
        let mut out = Vec::new();
        values.iter().for_each(|&value| write_int(&mut out, value));
        let mut reader = Reader::new(&out);
        let read: Vec<i64> = values.iter().map(|_| reader.int().unwrap()).collect();
        assert_eq!(read, values);
        assert!(reader.is_done());
        // RFC 8949 appendix A: -1000 and 1000000000000.
        let mut known = Vec::new();
        write_int(&mut known, -1000);
        write_int(&mut known, 1_000_000_000_000);
        assert_eq!(known, [0x39, 0x03, 0xe7, 0x1b, 0x00, 0x00, 0x00, 0xe8, 0xd4, 0xa5, 0x10, 0x00]);
    }

    #[test]
    fn an_array_longer_than_the_data_left_is_rejected() {
        // An array of 2^32 items in five bytes.
        let bytes = [0x9a, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(Reader::new(&bytes).array(), Err(WireError("truncated pack data".into())));
    }

    #[test]
    fn an_unsigned_integer_above_the_signed_range_is_an_error_not_a_wrap() {
        let mut out = Vec::new();
        write_uint(&mut out, u64::MAX);
        assert_eq!(Reader::new(&out).int(), Err(WireError("pack data: integer out of range".into())));
    }

    /// What the extractors' own decoder reads from `bytes`: an independent reading of what
    /// this module wrote.
    fn read_by_trace_data(bytes: &[u8]) -> f64 {
        match trace_data::cbor::Cursor::new(bytes).item() {
            Ok(trace_data::cbor::Item::Num(value)) => value,
            other => panic!("not a number: {other:?}"),
        }
    }

    #[test]
    fn every_finite_half_float_is_written_back_as_the_same_two_bytes() {
        for bits in 0..=u16::MAX {
            let half = [0xf9, (bits >> 8) as u8, bits as u8];
            let value = read_by_trace_data(&half);
            if value.is_finite() {
                assert_eq!(float_bytes(value), half, "half float {bits:#06x}");
            }
        }
    }

    #[test]
    fn a_float_of_any_width_reads_back_with_the_same_bits_and_is_no_wider_than_needed() {
        let mut state = 0x2026_1123_u64;
        for _ in 0..300_000 {
            // xorshift64: every bit pattern of a double and of a single gets its turn.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            for value in [f64::from_bits(state), f32::from_bits(state as u32) as f64] {
                if !value.is_finite() {
                    continue;
                }
                let bytes = float_bytes(value);
                assert_eq!(read_by_trace_data(&bytes).to_bits(), value.to_bits(), "{value:e}");
                // Eight bytes only for what a single cannot hold.
                assert_eq!(bytes.len() == 9, (value as f32) as f64 != value, "{value:e}");
            }
        }
    }
}
