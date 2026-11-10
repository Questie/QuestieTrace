//! Builds sessions for tests the way the TS tests write them: as JSON-like literals. They are
//! encoded to CBOR and decoded by `trace-data`, so observers are tested through the real reader.
//!
//! Lua tables are JSON objects with string keys: `{"1": "a", "n": 1}` is packed args, and a
//! key like "100" names the same property as the number 100.

use serde_json::{json, Value as Json};
use trace_data::value::decode_trace;
use trace_data::Session;

/// `{t, tp: t, e: name, a: args}`.
pub fn event(t: f64, name: &str, args: Json) -> Json {
    json!({ "t": t, "tp": t, "e": name, "a": args })
}

/// Packed args `{1: v1, 2: v2, n: count}`; null values are left out like Lua nil.
pub fn packed(values: &[Json]) -> Json {
    let mut map = serde_json::Map::new();
    for (i, value) in values.iter().enumerate() {
        if !value.is_null() {
            map.insert((i + 1).to_string(), value.clone());
        }
    }
    map.insert("n".into(), json!(values.len()));
    Json::Object(map)
}

/// A leaf stream: entries `{t, tp: t, v}` (a null `v` is left out, as nil).
pub fn stream(entries: impl IntoIterator<Item = (f64, Json)>) -> Json {
    Json::Array(
        entries
            .into_iter()
            .map(|(t, v)| {
                if v.is_null() {
                    json!({ "t": t, "tp": t })
                } else {
                    json!({ "t": t, "tp": t, "v": v })
                }
            })
            .collect(),
    )
}

/// A session object with `functions` and `events`.
pub fn session_json(functions: Json, events: Vec<Json>) -> Json {
    json!({ "startedAt": 0, "startedAtPrecise": 0, "events": events, "functions": functions })
}

/// Decodes `session` as the only session of an export and hands it to `f`.
///
/// An empty `functions` table decodes to an empty array, which is no valid session; tests
/// that need one get a placeholder function so the session stays valid.
pub fn with_session<R>(session: &Json, f: impl FnOnce(&Session) -> R) -> R {
    let mut session = session.clone();
    if session["functions"].as_object().is_some_and(serde_json::Map::is_empty) {
        session["functions"] = json!({ "_test": [] });
    }
    let bytes = encode(&json!({ "sessions": [session] }));
    let root = decode_trace(&bytes).expect("test session decodes").expect("not nil");
    let sessions = root.get("sessions").and_then(trace_data::Value::as_array).expect("sessions array");
    let session = Session::new(&sessions[0]).expect("a valid session");
    f(&session)
}

/// CBOR as Blizzard's serializer writes it, for the JSON subset tests use.
pub fn encode(value: &Json) -> Vec<u8> {
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
