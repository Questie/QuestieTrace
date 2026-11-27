//! CBOR built by hand for tests, one item at a time, so a test controls exactly which bytes
//! an export holds: the width of a float, an integer where a float is expected.

use crate::wire;

pub fn text(value: &str) -> Vec<u8> {
    let mut out = Vec::new();
    wire::write_text(&mut out, value);
    out
}

pub fn bytes(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    wire::write_bytes(&mut out, value);
    out
}

pub fn uint(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    wire::write_uint(&mut out, value);
    out
}

/// A float as the addon writes it: in the shortest width that holds it.
pub fn float(value: f64) -> Vec<u8> {
    let mut out = Vec::new();
    wire::write_float_shortest(&mut out, value);
    out
}

/// A float in eight bytes, whatever its value.
pub fn double(value: f64) -> Vec<u8> {
    let mut out = vec![0xfb];
    out.extend(value.to_be_bytes());
    out
}

/// A float in four bytes.
pub fn single(value: f32) -> Vec<u8> {
    let mut out = vec![0xfa];
    out.extend(value.to_be_bytes());
    out
}

/// A float in two bytes, from its bits.
pub fn half(bits: u16) -> Vec<u8> {
    let mut out = vec![0xf9];
    out.extend(bits.to_be_bytes());
    out
}

pub fn array(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    wire::write_array_head(&mut out, items.len());
    items.iter().for_each(|item| out.extend_from_slice(item));
    out
}

/// A map of at most 23 pairs.
pub fn map(pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    assert!(pairs.len() < 24, "test maps keep their length in the initial byte");
    let mut out = vec![0xa0 | pairs.len() as u8];
    for (key, value) in pairs {
        out.extend_from_slice(key);
        out.extend_from_slice(value);
    }
    out
}
