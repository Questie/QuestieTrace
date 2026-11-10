//! Export strings to CBOR bytes, reversing Modules/Export/Encoding.lua:
//!
//!   "!QuestieTrace:N!" + EncodeForPrint(raw deflate(CBOR(payload))) + "!End:QuestieTrace:N!"
//!
//! Mirrors `core/trace-data/decode.ts`, including its error messages (they end up in
//! meta.json). Inflating uses libdeflate; only when it rejects a payload is zlib asked why, so
//! the message is the one the TS tool (Node's zlib) reports.

use libdeflater::{DecompressionError, Decompressor};

const PRINT_ALPHABET: &[u8; 64] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789()";

/// Far above any real export (the largest inflate to a few dozen MB); stops a hostile payload
/// from exhausting memory.
const MAX_CBOR_BYTES: usize = 512 * 1024 * 1024;

const SIX_BIT: [i8; 256] = {
    let mut table = [-1i8; 256];
    let mut i = 0;
    while i < 64 {
        table[PRINT_ALPHABET[i] as usize] = i as i8;
        i += 1;
    }
    table
};

/// The exports of one `export_string`, in order of their start markers.
pub enum Export<'s> {
    /// The text between a start marker and its matching end marker.
    Payload(&'s str),
    /// A start marker without a matching end marker.
    Unterminated,
}

const START: &str = "!QuestieTrace:";
const END: &str = "!End:QuestieTrace:";

/// Every match of `/!QuestieTrace:\d+!/g`: its start offset and the end of the match.
fn start_markers(s: &str) -> Vec<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(rel) = memchr::memmem::find(&bytes[from..], START.as_bytes()) {
        let at = from + rel;
        let digits = at + START.len();
        let digit_count = bytes[digits..].iter().take_while(|b| b.is_ascii_digit()).count();
        if digit_count > 0 && bytes.get(digits + digit_count) == Some(&b'!') {
            let end = digits + digit_count + 1;
            found.push((at, end));
            from = end;
        } else {
            from = at + 1;
        }
    }
    found
}

/// Every match of `/!QuestieTrace:(\d+)!([^!]*)!End:QuestieTrace:\1!/g`: its start offset and
/// payload. The payload cannot hold a `!`, so it runs to the next one.
fn complete_exports(s: &str) -> Vec<(usize, &str)> {
    let bytes = s.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(rel) = memchr::memmem::find(&bytes[from..], START.as_bytes()) {
        let at = from + rel;
        let digits = at + START.len();
        let digit_count = bytes[digits..].iter().take_while(|b| b.is_ascii_digit()).count();
        let version = &s[digits..digits + digit_count];
        let payload_start = digits + digit_count + 1;
        let matched = (digit_count > 0 && bytes.get(digits + digit_count) == Some(&b'!'))
            .then(|| {
                let payload_end = memchr::memchr(b'!', &bytes[payload_start..]).map(|i| payload_start + i)?;
                let rest = &s[payload_end..];
                let closes = rest.strip_prefix(END)?.strip_prefix(version)?.starts_with('!');
                closes.then_some((payload_end, payload_end + END.len() + version.len() + 1))
            })
            .flatten();
        match matched {
            Some((payload_end, end)) => {
                found.push((at, &s[payload_start..payload_end]));
                from = end;
            }
            None => from = at + 1,
        }
    }
    found
}

/// The exports in `export_string`, or the error decode.ts throws when there is none.
pub fn find_exports(export_string: &str) -> Result<Vec<Export<'_>>, String> {
    let starts = start_markers(export_string);
    if starts.is_empty() {
        return Err("export string must be wrapped in !QuestieTrace:N! and !End:QuestieTrace:N! markers".into());
    }
    let complete = complete_exports(export_string);
    Ok(starts
        .iter()
        .map(|(at, _)| match complete.iter().find(|(start, _)| start == at) {
            Some((_, payload)) => Export::Payload(payload),
            None => Export::Unterminated,
        })
        .collect())
}

/// JS `\s`: the characters `/\s+/g` removes.
pub(crate) fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// Reverses LibDeflate:EncodeForPrint: 4 characters carry 3 bytes, least significant 6 bits
/// first; a short tail carries the remaining whole bytes. Whitespace is ignored anywhere.
pub fn decode_for_print(text: &str) -> Result<Vec<u8>, String> {
    if let Some(out) = decode_clean_print(text.as_bytes()) {
        return Ok(out);
    }
    // TS counts and quotes UTF-16 units: a character outside the BMP is two of them.
    let kept: usize = text.chars().filter(|&c| !is_js_whitespace(c)).map(char::len_utf16).sum();
    if kept == 1 {
        return Err("payload is not valid print-encoded data".into());
    }
    let mut out = Vec::with_capacity(kept * 6 / 8);
    let mut cache: u32 = 0;
    let mut bits: u32 = 0;
    for c in text.chars().filter(|&c| !is_js_whitespace(c)) {
        let value = if (c as u32) < 256 { SIX_BIT[c as usize] } else { -1 };
        if value < 0 {
            // JSON.stringify of the UTF-16 unit: a high surrogate alone prints escaped.
            let quoted = match c.encode_utf16(&mut [0; 2]) {
                [high, _] => format!("\"\\u{high:04x}\""),
                _ => serde_json::to_string(&c.to_string()).expect("a string always serializes"),
            };
            return Err(format!("payload is not valid print-encoded data (character {quoted})"));
        }
        cache |= (value as u32) << bits;
        bits += 6;
        if bits >= 8 {
            out.push(cache as u8);
            cache >>= 8;
            bits -= 8;
        }
    }
    Ok(out)
}

/// The common case: a payload of alphabet characters only, decoded 4 characters (3 bytes) at a
/// time. None when anything else (whitespace, an invalid character) or a single character needs
/// the general path above.
fn decode_clean_print(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() <= 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    let mut chunks = bytes.chunks_exact(4);
    for chunk in &mut chunks {
        let [a, b, c, d] = [chunk[0], chunk[1], chunk[2], chunk[3]].map(|byte| SIX_BIT[byte as usize]);
        if (a | b | c | d) < 0 {
            return None;
        }
        let bits = a as u32 | (b as u32) << 6 | (c as u32) << 12 | (d as u32) << 18;
        out.extend_from_slice(&[bits as u8, (bits >> 8) as u8, (bits >> 16) as u8]);
    }
    let (mut cache, mut bits) = (0u32, 0u32);
    for &byte in chunks.remainder() {
        let value = SIX_BIT[byte as usize];
        if value < 0 {
            return None;
        }
        cache |= (value as u32) << bits;
        bits += 6;
        if bits >= 8 {
            out.push(cache as u8);
            cache >>= 8;
            bits -= 8;
        }
    }
    Some(out)
}

/// An inflated export: the first `size` bytes of `buffer`. The buffer keeps its full length so
/// that, recycled into an [`Inflater`], it is reused without being cleared.
pub struct Inflated {
    buffer: Vec<u8>,
    size: usize,
}

impl std::fmt::Debug for Inflated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Inflated({} bytes)", self.size)
    }
}

impl Inflated {
    pub fn bytes(&self) -> &[u8] {
        &self.buffer[..self.size]
    }
}

/// Raw-deflate inflating into recycled buffers: an export is inflated once, straight into a
/// buffer an earlier export left behind (see [`Inflater::recycle`]), so neither copying nor
/// clearing memory is needed in the common case.
#[derive(Default)]
pub struct Inflater {
    decompressor: Option<Decompressor>,
    spare: Vec<Vec<u8>>,
}

impl Inflater {
    pub fn new() -> Self {
        Inflater {
            decompressor: Some(Decompressor::new()),
            spare: Vec::new(),
        }
    }

    /// Hands buffers back for later exports.
    pub fn recycle(&mut self, inflated: impl IntoIterator<Item = Inflated>) {
        self.spare.extend(inflated.into_iter().map(|inflated| inflated.buffer));
    }

    pub fn inflate(&mut self, compressed: &[u8]) -> Result<Inflated, String> {
        let wanted = (compressed.len() * 8).clamp(64 * 1024, MAX_CBOR_BYTES);
        let mut buffer = self.spare.pop().unwrap_or_default();
        if buffer.len() < wanted {
            buffer.resize(wanted, 0);
        }
        let decompressor = self.decompressor.get_or_insert_with(Decompressor::new);
        loop {
            match decompressor.deflate_decompress(compressed, &mut buffer) {
                Ok(size) => return Ok(Inflated { buffer, size }),
                Err(DecompressionError::InsufficientSpace) if buffer.len() < MAX_CBOR_BYTES => {
                    let grown = (buffer.len() * 2).min(MAX_CBOR_BYTES);
                    buffer.resize(grown, 0);
                }
                Err(error) => {
                    self.spare.push(buffer);
                    return Err(match error {
                        DecompressionError::InsufficientSpace => {
                            format!("payload could not be decompressed: Cannot create a Buffer larger than {MAX_CBOR_BYTES} bytes")
                        }
                        DecompressionError::BadData => format!("payload could not be decompressed: {}", zlib_error(compressed)),
                    });
                }
            }
        }
    }
}

/// Why zlib, which the TS tool inflates with, rejects data libdeflate rejected: zlib's own
/// message, or Node's "unexpected end of file" when the input ends inside the stream. Only the
/// error path pays for inflating twice.
fn zlib_error(compressed: &[u8]) -> String {
    use flate2::{Decompress, FlushDecompress, Status};
    let mut zlib = Decompress::new(false);
    let mut out: Vec<u8> = Vec::with_capacity(compressed.len() * 8);
    loop {
        if out.len() == out.capacity() {
            out.reserve(out.capacity().max(64 * 1024));
        }
        let (before_in, before_out) = (zlib.total_in(), zlib.total_out());
        let rest = &compressed[before_in as usize..];
        // No-flush calls: zlib-rs refuses a second Finish call after one ran out of output space.
        match zlib.decompress_vec(rest, &mut out, FlushDecompress::None) {
            Err(e) => return e.message().unwrap_or("invalid deflate data").to_owned(),
            // zlib accepts what libdeflate rejected: keep libdeflate's verdict.
            Ok(Status::StreamEnd) => return "invalid deflate data".into(),
            Ok(_) if zlib.total_in() == before_in && zlib.total_out() == before_out && out.len() < out.capacity() => {
                return "unexpected end of file".into();
            }
            Ok(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<String> {
        find_exports(s)
            .unwrap()
            .iter()
            .map(|e| match e {
                Export::Payload(p) => p.to_string(),
                Export::Unterminated => "unterminated".into(),
            })
            .collect()
    }

    #[test]
    fn print_decoding_reverses_known_bytes_and_ignores_whitespace() {
        // "abc" is 0x636261 read little-endian; its 6-bit groups, least significant first,
        // are 33, 9, 54, 24: alphabet characters H, j, 2, y.
        assert_eq!(decode_for_print("Hj2y").unwrap(), b"abc");
        assert_eq!(decode_for_print(" H j\n2\u{a0}y ").unwrap(), b"abc");
    }

    #[test]
    fn the_clean_fast_path_agrees_with_the_general_path_for_every_tail_length() {
        let alphabet = std::str::from_utf8(PRINT_ALPHABET).unwrap();
        for length in 2..40 {
            let text: String = alphabet.chars().cycle().skip(length * 7).take(length).collect();
            // Interleaved whitespace forces the general path over the same characters.
            let spaced: String = text.chars().flat_map(|c| [c, ' ']).collect();
            assert_eq!(decode_for_print(&text).unwrap(), decode_for_print(&spaced).unwrap(), "length {length}");
        }
    }

    #[test]
    fn characters_outside_the_bmp_count_and_quote_as_utf16_units_like_ts() {
        // node: decodeForPrint("abc😀d") names "\ud83d"; a lone "😀" is two units, so not "one character".
        assert_eq!(
            decode_for_print("abc😀d").unwrap_err(),
            "payload is not valid print-encoded data (character \"\\ud83d\")"
        );
        assert_eq!(
            decode_for_print("😀").unwrap_err(),
            "payload is not valid print-encoded data (character \"\\ud83d\")"
        );
    }

    #[test]
    fn print_decoding_rejects_a_single_character_and_foreign_characters() {
        assert_eq!(decode_for_print(" a ").unwrap_err(), "payload is not valid print-encoded data");
        assert_eq!(
            decode_for_print("ab*c").unwrap_err(),
            "payload is not valid print-encoded data (character \"*\")"
        );
    }

    #[test]
    fn an_export_pasted_into_another_is_matched_and_the_cut_off_one_is_unterminated() {
        let s = "!QuestieTrace:1!aa!QuestieTrace:1!bb!End:QuestieTrace:1! !QuestieTrace:2!cc!End:QuestieTrace:1!";
        assert_eq!(kinds(s), ["unterminated", "bb", "unterminated"]);
    }

    #[test]
    fn the_end_marker_must_repeat_the_start_version() {
        assert_eq!(kinds("!QuestieTrace:12!ab!End:QuestieTrace:12!"), ["ab"]);
        assert_eq!(kinds("!QuestieTrace:1!ab!End:QuestieTrace:12!"), ["unterminated"]);
    }

    #[test]
    fn a_recycled_buffer_inflates_a_smaller_export_without_its_stale_bytes() {
        let compress = |data: &[u8]| {
            let mut compressor = libdeflater::Compressor::new(libdeflater::CompressionLvl::default());
            let mut out = vec![0; compressor.deflate_compress_bound(data.len())];
            let size = compressor.deflate_compress(data, &mut out).unwrap();
            out.truncate(size);
            out
        };
        let large: Vec<u8> = (0..200_000u32).map(|i| (i % 7) as u8 + 1).collect();
        let small = b"short export".to_vec();
        let mut inflater = Inflater::new();
        let first = inflater.inflate(&compress(&large)).unwrap();
        assert_eq!(first.bytes(), large);
        inflater.recycle([first]);
        assert_eq!(inflater.inflate(&compress(&small)).unwrap().bytes(), small);
    }

    #[test]
    fn corrupt_and_cut_off_deflate_data_get_zlibs_messages() {
        let mut compressor = libdeflater::Compressor::new(libdeflater::CompressionLvl::default());
        let data: Vec<u8> = (0..20_000u32).flat_map(|i| (i % 251).to_le_bytes()).collect();
        let mut compressed = vec![0; compressor.deflate_compress_bound(data.len())];
        let size = compressor.deflate_compress(&data, &mut compressed).unwrap();
        compressed.truncate(size);
        let mut inflater = Inflater::new();
        assert_eq!(inflater.inflate(&compressed).unwrap().bytes(), data);
        // Node: "unexpected end of file" for input that ends inside the stream.
        assert_eq!(
            inflater.inflate(&compressed[..size / 2]).unwrap_err(),
            "payload could not be decompressed: unexpected end of file"
        );
        // A reserved block type (0b11) is invalid in every deflate decoder; zlib names it.
        assert_eq!(
            inflater.inflate(&[0x07, 0x00]).unwrap_err(),
            "payload could not be decompressed: invalid block type"
        );
    }

    #[test]
    fn no_start_marker_is_an_error() {
        let error = find_exports("QuestieTrace:1!ab!End:QuestieTrace:1!").err();
        assert_eq!(
            error.as_deref(),
            Some("export string must be wrapped in !QuestieTrace:N! and !End:QuestieTrace:N! markers")
        );
    }
}
