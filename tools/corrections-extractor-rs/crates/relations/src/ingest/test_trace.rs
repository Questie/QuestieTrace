//! Raw sessions for ingest tests, written as JSON literals the way the TS tests write them. They
//! are encoded to CBOR and decoded by `trace-data`, so distill is tested through the real reader.
//!
//! Lua tables are JSON objects with string keys: `{"1": "a", "n": 1}` is packed args, and a key
//! like "100" names the same property as the number 100. JSON null stands for Lua nil.

use super::distill::{distill_session, episode_key, DistilledSession};
use serde_json::{json, Value as Json};
use trace_data::value::decode_trace;
use trace_data::{Arena, Session, Value};

/// Decodes `value` through the real decoder and hands the result to `f`.
pub fn decode_json<R>(value: &Json, f: impl FnOnce(&Value) -> R) -> R {
    let bytes = encode(value);
    let arena = Arena::new();
    let decoded = decode_trace(&bytes, &arena).expect("test value decodes").expect("not nil");
    f(&decoded)
}

/// Decodes `records` as the `sessions` array of one export and hands the decoded records to `f`.
pub fn with_records<R>(records: &[Json], f: impl FnOnce(&[Value]) -> R) -> R {
    decode_json(&json!({ "sessions": records }), |root| {
        f(root.get("sessions").and_then(Value::as_array).expect("a sessions array"))
    })
}

/// Distills `record`, which must be a valid session record.
pub fn distill_json(record: &Json) -> DistilledSession {
    with_records(std::slice::from_ref(record), |records| {
        let session = Session::new(&records[0]).expect("a valid session record");
        distill_session(&session, episode_key(&records[0]))
    })
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

pub fn creature(id: i64) -> String {
    format!("Creature-0-1-2-3-{id}-0000AB")
}

/// A gossip list entry as C_GossipInfo returns it.
pub fn gossip_quest(quest_id: i64) -> Json {
    json!({ "questID": quest_id, "title": format!("Quest {quest_id}"), "isTrivial": false, "frequency": 0, "repeatable": false })
}

/// Builds a raw session the way the addon records one: everything inside one frame shares `t`,
/// while `tp` increases with every event and sample, and samples follow the event they react to.
/// A level-less Horde orc warrior on an enUS client, with capture-start clocks.
pub struct TraceBuilder {
    frame_t: f64,
    tp: f64,
    pub session: Json,
}

impl TraceBuilder {
    pub fn new() -> Self {
        let player = |v: Json| json!({ "player": [{ "t": 0, "tp": 0, "v": v }] });
        TraceBuilder {
            frame_t: 0.0,
            tp: 0.0,
            session: json!({
                "schemaVersion": 12,
                "startedAt": 1000,
                "startedAtPrecise": 2000.5,
                "events": [],
                "functions": {
                    "UnitRace": player(json!({ "1": "Orc", "2": "Orc", "3": 2, "n": 3 })),
                    "UnitClass": player(json!({ "1": "Warrior", "2": "WARRIOR", "3": 1, "n": 3 })),
                    "UnitFactionGroup": player(json!({ "1": "Horde", "2": "Horde", "n": 2 })),
                    "GetLocale": player(json!("enUS")),
                },
                "functionsDelta": {},
            }),
        }
    }

    pub fn frame(mut self, t: f64) -> Self {
        self.frame_t = t;
        self.tp = self.tp.max(t);
        self
    }

    /// An event with packed args; null args are nil.
    pub fn event(mut self, name: &str, args: &[Json]) -> Self {
        let mut a = serde_json::Map::new();
        for (index, value) in args.iter().enumerate().filter(|(_, value)| !value.is_null()) {
            a.insert((index + 1).to_string(), value.clone());
        }
        a.insert("n".into(), json!(args.len()));
        let tp = self.next_tp();
        let event = json!({ "t": self.frame_t, "tp": tp, "e": name, "a": a });
        self.session["events"].as_array_mut().expect("events").push(event);
        self
    }

    /// A sample of a parameterless stream; null records a nil read.
    pub fn sample(self, name: &str, v: Json) -> Self {
        self.sample_path(&[name], v, None)
    }

    /// A sample of `functions[name][param]`.
    pub fn sample_param(self, name: &str, param: &str, v: Json) -> Self {
        self.sample_path(&[name, param], v, None)
    }

    fn sample_path(mut self, path: &[&str], v: Json, tp: Option<f64>) -> Self {
        let tp = tp.unwrap_or_else(|| self.next_tp());
        let mut node = &mut self.session["functions"];
        for (depth, key) in path.iter().enumerate() {
            let leaf = depth == path.len() - 1;
            node = node
                .as_object_mut()
                .expect("a parameterized stream")
                .entry(*key)
                .or_insert_with(|| if leaf { json!([]) } else { json!({}) });
        }
        let entry = if v.is_null() {
            json!({ "t": self.frame_t, "tp": tp })
        } else {
            json!({ "t": self.frame_t, "tp": tp, "v": v })
        };
        node.as_array_mut().expect("a leaf stream").push(entry);
        self
    }

    /// QuestDialog reads GetQuestID and GetTitleText in one call, so both samples share a `tp`.
    pub fn quest_dialog_read(mut self, quest_id: i64, title: &str) -> Self {
        let tp = self.next_tp();
        self.sample_path(&["GetQuestID"], json!(quest_id), Some(tp))
            .sample_path(&["GetTitleText"], json!(title), Some(tp))
    }

    /// The samples UnitInteraction and QuestDialog take when a dialog opens at `npc_id`.
    pub fn dialog(self, name: &str, quest_id: i64, npc_id: Option<i64>, args: &[Json]) -> Self {
        let guid = npc_id.map_or(Json::Null, |id| json!(creature(id)));
        self.event(name, args)
            .sample("GetQuestID", json!(quest_id))
            .sample_param("UnitGUID", "npc", guid.clone())
            .sample_param("UnitGUID", "questnpc", guid)
    }

    /// QUEST_FINISHED with the closed-state samples.
    pub fn close(self) -> Self {
        self.event("QUEST_FINISHED", &[])
            .sample("GetQuestID", json!(0))
            .sample_param("UnitGUID", "npc", Json::Null)
            .sample_param("UnitGUID", "questnpc", Json::Null)
    }

    pub fn gossip(self, npc_id: i64, available: &[Json], active: &[Json]) -> Self {
        self.event("GOSSIP_SHOW", &[Json::Null])
            .sample_param("UnitGUID", "npc", json!(creature(npc_id)))
            .sample("C_GossipInfo.GetNumAvailableQuests", json!(available.len()))
            .sample("C_GossipInfo.GetAvailableQuests", json!(available))
            .sample("C_GossipInfo.GetActiveQuests", json!(active))
    }

    pub fn build(&self) -> DistilledSession {
        distill_json(&self.session)
    }

    fn next_tp(&mut self) -> f64 {
        self.tp = ((self.tp + 0.001) * 1e6).round() / 1e6;
        self.tp
    }
}
