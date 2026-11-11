//! Point-in-time reads over raw session streams, ordered by `tp` (GetTimePreciseSec).
//!
//! Why not `t`: GetTime() is cached per frame, so a QUEST_DETAIL, the QUEST_FINISHED that closes
//! it and both of their samples can all share one `t`. Looking a value up "at t" then returns the
//! *last* sample of the frame (often the closed state: GetQuestID = 0). `tp` is unique per call
//! and the core records an event before dispatching it to trackers, so the samples taken for
//! event i lie between events[i].tp and events[i + 1].tp.

use trace_data::stream::{entry_t, entry_v, last_at_or_before, Stream};
use trace_data::{Event, Value};

/// An entry's `tp`, falling back to `t` where `tp` was not recorded.
pub fn entry_time(entry: &Value) -> f64 {
    entry.get("tp").and_then(Value::as_num).unwrap_or_else(|| entry_t(entry))
}

/// [`entry_time`] for an event.
pub fn event_time(event: &Event) -> f64 {
    if event.tp().is_nan() {
        event.t()
    } else {
        event.tp()
    }
}

/// Index of the last entry with time < `before` (or <= with `inclusive`). Streams are appended
/// in time order.
pub fn last_index_before(stream: Stream, before: f64, inclusive: bool) -> Option<usize> {
    last_at_or_before(stream.len(), |i| {
        let time = entry_time(&stream[i]);
        time < before || (inclusive && time == before)
    })
}

/// The value in effect just before `before` (Lua nil and "never sampled" are both None).
pub fn value_before<'v, 'a>(stream: Option<Stream<'v, 'a>>, before: f64) -> Option<&'v Value<'a>> {
    let stream = stream?;
    entry_v(&stream[last_index_before(stream, before, false)?])
}

/// The value of the sample taken at `time` or the latest one before it.
pub fn value_at_or_before<'v, 'a>(stream: Option<Stream<'v, 'a>>, time: f64) -> Option<&'v Value<'a>> {
    let stream = stream?;
    entry_v(&stream[last_index_before(stream, time, true)?])
}

/// Whether the stream has any sample before `before`: a read happened, even if it returned nil.
pub fn sampled_before(stream: Option<Stream>, before: f64) -> bool {
    stream.is_some_and(|stream| last_index_before(stream, before, false).is_some())
}

/// The entries with `from <= time < to`.
pub fn entries_between<'v, 'a>(stream: Option<Stream<'v, 'a>>, from: f64, to: f64) -> Stream<'v, 'a> {
    let Some(stream) = stream else { return &[] };
    let Some(end) = last_index_before(stream, to, false) else { return &[] };
    let count = stream[..=end].iter().rev().take_while(|entry| entry_time(entry) >= from).count();
    &stream[end + 1 - count..=end]
}

/// Packed args or packed returns, `{1: a, 2: b, n: 2}`: the value at 1-based `index`. Read from
/// an array too, whose items sit at 0-based positions.
pub fn packed<'v, 'a>(value: Option<&'v Value<'a>>, index: u32) -> Option<&'v Value<'a>> {
    let value = value?;
    match value.as_array() {
        Some(items) => items.get(index.checked_sub(1)? as usize),
        None => value.get(index),
    }
}

pub fn event_arg<'v, 'a>(event: &Event<'v, 'a>, index: u32) -> Option<&'v Value<'a>> {
    packed(event.args(), index)
}

/// Largest number accepted as an id: above it, an `f64` no longer converts to `i64` exactly.
/// Quest, NPC and item ids are far below.
const MAX_ID: f64 = 9.0e18;

/// A positive integer (quest, NPC, item id or count), else None.
pub fn positive_int(n: f64) -> Option<i64> {
    (n.fract() == 0.0 && n > 0.0 && n <= MAX_ID).then_some(n as i64)
}

/// A positive integer number value, else None.
pub fn as_positive_int(value: Option<&Value>) -> Option<i64> {
    positive_int(value?.as_num()?)
}

/// Any integer number, for small enum-like values the contracts type as integers (offer
/// frequency, quest classification). The client only returns integers there.
pub fn as_integer(value: Option<&Value>) -> Option<i64> {
    let n = value?.as_num()?;
    (n.fract() == 0.0 && n.abs() <= MAX_ID).then_some(n as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::test_trace::decode_json;
    use serde_json::json;

    // Samples for an event are taken right after it, so "before the next event" must exclude a
    // sample stamped exactly at that next event's tp, and "at this read" must include its own tp.
    fn with_stream(entries: serde_json::Value, check: impl FnOnce(Stream)) {
        decode_json(&entries, |value| check(value.as_array().expect("a stream")));
    }

    fn text<'v>(value: Option<&'v Value>) -> Option<&'v str> {
        value.and_then(Value::as_str)
    }

    #[test]
    fn value_before_is_strict_and_value_at_or_before_inclusive_and_a_nil_read_is_still_a_read() {
        let entries = json!([
            { "t": 1, "tp": 1, "v": "a" },
            { "t": 2, "tp": 2, "v": "b" },
            { "t": 2, "tp": 2.5 },
            { "t": 3, "tp": 3, "v": "c" },
        ]);
        with_stream(entries, |stream| {
            assert_eq!(text(value_before(Some(stream), 2.0)), Some("a"));
            assert_eq!(text(value_at_or_before(Some(stream), 2.0)), Some("b"));
            assert_eq!(text(value_before(Some(stream), 1.0)), None);
            let tps: Vec<f64> = entries_between(Some(stream), 2.0, 3.0).iter().map(entry_time).collect();
            assert_eq!(tps, [2.0, 2.5]);
            // The nil read at 2.5 is in effect at 2.9, and it was a read.
            assert_eq!(text(value_before(Some(stream), 2.9)), None);
            assert!(sampled_before(Some(stream), 2.9));
            assert!(!sampled_before(Some(stream), 0.5));
        });
    }

    #[test]
    fn orders_by_tp_falling_back_to_t_only_where_tp_is_missing() {
        let frame = json!([{ "t": 5, "tp": 5.2, "v": "early" }, { "t": 5, "tp": 5.4, "v": "late" }]);
        with_stream(frame, |stream| assert_eq!(text(value_before(Some(stream), 5.3)), Some("early")));
        with_stream(json!([{ "t": 5, "v": "legacy" }]), |stream| {
            assert_eq!(text(value_before(Some(stream), 5.1)), Some("legacy"))
        });
    }

    #[test]
    fn packed_reads_one_based_from_objects_and_arrays() {
        decode_json(&json!({ "1": "Orc", "3": 2, "n": 3 }), |value| {
            assert_eq!(text(packed(Some(value), 1)), Some("Orc"));
            assert!(packed(Some(value), 2).is_none(), "a nil hole");
            assert_eq!(as_positive_int(packed(Some(value), 3)), Some(2));
        });
        // A packed table without holes or `n` decodes to an array: index 3 is its third item.
        decode_json(&json!(["Orc", "Orc", 2]), |value| {
            assert_eq!(as_positive_int(packed(Some(value), 3)), Some(2));
            assert!(packed(Some(value), 0).is_none());
        });
    }
}
