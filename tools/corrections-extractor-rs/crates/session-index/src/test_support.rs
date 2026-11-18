//! Builds sessions and submissions for tests as JSON literals, encoded the way the addon encodes
//! them (CBOR, raw deflate, print alphabet), so tests read them through the real decoder.
//!
//! The encoder is the one `corrections` and `relations` carry in their own test helpers.

use serde_json::{json, Value as Json};
use trace_data::value::decode_trace;
use trace_data::{Arena, Session, SessionKey, Value};

/// The key of a session that started at these clocks, as the real reader computes it.
pub fn session_key(started_at: f64, started_at_precise: f64) -> SessionKey {
    let record = session(Some((started_at, started_at_precise)), 0);
    let bytes = encode(&json!({ "sessions": [record] }));
    let arena = Arena::new();
    let root = decode_trace(&bytes, &arena).expect("decodes").expect("not nil");
    let sessions = root.get("sessions").and_then(Value::as_array).expect("a sessions array");
    Session::new(&sessions[0]).expect("a valid session").key().expect("both clocks")
}

/// A valid session record with `events` events, started at `clocks` (None: no clocks at all).
pub fn session(clocks: Option<(f64, f64)>, events: usize) -> Json {
    let events: Vec<Json> = (0..events).map(|i| json!({ "t": i, "tp": i, "e": "PLAYER_LOGIN", "a": { "n": 0 } })).collect();
    // An empty `functions` table decodes to an array, which makes no valid session.
    let mut record = json!({ "events": events, "functions": { "GetLocale": [{ "t": 0, "tp": 0, "v": "enUS" }] } });
    if let Some((started_at, started_at_precise)) = clocks {
        record["startedAt"] = json!(started_at);
        record["startedAtPrecise"] = json!(started_at_precise);
    }
    record
}

/// An addon export (`!QuestieTrace:1!...!End:QuestieTrace:1!`) holding `sessions`.
pub fn export_string(sessions: &[Json]) -> String {
    let cbor = encode(&json!({ "sessions": sessions }));
    let mut compressor = libdeflater::Compressor::new(libdeflater::CompressionLvl::default());
    let mut compressed = vec![0; compressor.deflate_compress_bound(cbor.len())];
    let size = compressor.deflate_compress(&cbor, &mut compressed).expect("compresses");
    compressed.truncate(size);
    format!("!QuestieTrace:1!{}!End:QuestieTrace:1!", encode_for_print(&compressed))
}

/// CBOR as Blizzard's serializer writes it, for the JSON subset tests use.
fn encode(value: &Json) -> Vec<u8> {
    let mut out = Vec::new();
    write(&mut out, value);
    out
}

fn header(out: &mut Vec<u8>, major: u8, n: u64) {
    let major = major << 5;
    match n {
        0..=23 => out.push(major | n as u8),
        24..=0xff => out.extend([major | 24, n as u8]),
        0x100..=0xffff => {
            out.push(major | 25);
            out.extend((n as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(major | 26);
            out.extend((n as u32).to_be_bytes());
        }
        _ => {
            out.push(major | 27);
            out.extend(n.to_be_bytes());
        }
    }
}

fn write(out: &mut Vec<u8>, value: &Json) {
    match value {
        Json::Null => out.push(0xf6),
        Json::Bool(false) => out.push(0xf4),
        Json::Bool(true) => out.push(0xf5),
        Json::Number(n) => match (n.as_u64(), n.as_i64()) {
            (Some(u), _) => header(out, 0, u),
            (None, Some(i)) => header(out, 1, (-1 - i) as u64),
            _ => {
                out.push(0xfb);
                out.extend(n.as_f64().expect("a JSON number").to_be_bytes());
            }
        },
        Json::String(s) => {
            header(out, 3, s.len() as u64);
            out.extend(s.as_bytes());
        }
        Json::Array(items) => {
            header(out, 4, items.len() as u64);
            for item in items {
                write(out, item);
            }
        }
        Json::Object(map) => {
            header(out, 5, map.len() as u64);
            for (key, item) in map {
                write(out, &Json::String(key.clone()));
                write(out, item);
            }
        }
    }
}

const PRINT_ALPHABET: &[u8; 64] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789()";

/// LibDeflate:EncodeForPrint: 3 bytes in 4 characters, least significant 6 bits first.
fn encode_for_print(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().rev().fold(0u32, |acc, &b| (acc << 8) | b as u32);
        let characters = (chunk.len() * 8).div_ceil(6);
        for i in 0..characters {
            out.push(PRINT_ALPHABET[(bits >> (6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// A directory removed again when dropped.
pub struct TempDir(pub std::path::PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let unique = format!("session-index-{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("creates the temp dir");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
