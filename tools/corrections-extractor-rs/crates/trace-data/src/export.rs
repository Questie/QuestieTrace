//! Export strings to CBOR bytes, reversing Modules/Export/Encoding.lua:
//!
//!   "!QuestieTrace:N!" + EncodeForPrint(raw deflate(CBOR(payload))) + "!End:QuestieTrace:N!"
//!
//! Mirrors `core/trace-data/decode.ts`, including its error messages (they end up in
//! meta.json), except where zlib words an inflate error differently from libdeflate.

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
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// Reverses LibDeflate:EncodeForPrint: 4 characters carry 3 bytes, least significant 6 bits
/// first; a short tail carries the remaining whole bytes. Whitespace is ignored anywhere.
pub fn decode_for_print(text: &str) -> Result<Vec<u8>, String> {
    let kept = text.chars().filter(|&c| !is_js_whitespace(c)).count();
    if kept == 1 {
        return Err("payload is not valid print-encoded data".into());
    }
    let mut out = Vec::with_capacity(kept * 6 / 8);
    let mut cache: u32 = 0;
    let mut bits: u32 = 0;
    for c in text.chars().filter(|&c| !is_js_whitespace(c)) {
        let value = if (c as u32) < 256 { SIX_BIT[c as usize] } else { -1 };
        if value < 0 {
            let quoted = serde_json::to_string(&c.to_string()).expect("a string always serializes");
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

/// Inflates raw deflate data.
pub fn inflate(decompressor: &mut Decompressor, compressed: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; (compressed.len() * 8).clamp(64 * 1024, MAX_CBOR_BYTES)];
    loop {
        match decompressor.deflate_decompress(compressed, &mut out) {
            Ok(size) => {
                out.truncate(size);
                return Ok(out);
            }
            Err(DecompressionError::InsufficientSpace) if out.len() < MAX_CBOR_BYTES => {
                let grown = (out.len() * 2).min(MAX_CBOR_BYTES);
                out.resize(grown, 0);
            }
            Err(DecompressionError::InsufficientSpace) => {
                return Err(format!("payload could not be decompressed: Cannot create a Buffer larger than {MAX_CBOR_BYTES} bytes"));
            }
            Err(DecompressionError::BadData) => return Err("payload could not be decompressed: invalid deflate data".into()),
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
    fn print_decoding_rejects_a_single_character_and_foreign_characters() {
        assert_eq!(decode_for_print(" a ").unwrap_err(), "payload is not valid print-encoded data");
        assert_eq!(decode_for_print("ab*c").unwrap_err(), "payload is not valid print-encoded data (character \"*\")");
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
    fn no_start_marker_is_an_error() {
        assert!(find_exports("QuestieTrace:1!ab").is_err());
    }
}
