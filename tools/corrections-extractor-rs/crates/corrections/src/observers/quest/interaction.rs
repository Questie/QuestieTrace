//! Port of `extract/observers/quest/_interaction.ts`: reads API streams as they stood during one
//! quest event (QUEST_DETAIL, QUEST_ACCEPTED, GOSSIP_SHOW, ...), so a quest giver is taken from
//! that interaction and not from a neighbouring one.
//!
//! An event's synchronous probes are recorded after its raw timestamp, so a lookup includes
//! samples of the same frame up to the next event's precise time, and excludes everything from
//! the next event on. Events whose timestamps cannot be told apart get no lookup at all.
//!
//! Times are compared like JS numbers. A missing `t`/`tp` reads as NaN, where TS compares
//! `undefined` (and `undefined === undefined`); the addon always records both.

use trace_data::stream::{entry_t, entry_v, get_stream};
use trace_data::{KeyRef, Session, Value};

/// What a quest interaction saw of one API stream.
#[derive(Clone, Copy, Debug)]
pub enum InteractionValue<'v, 'a> {
    /// The latest sample within the interaction's time boundary; None when there is none or it
    /// recorded nil. A recorded nil is kept: older samples are never searched.
    Known(Option<&'v Value<'a>>),
    /// The latest sample ties with the next event's timestamp, so it may belong to either
    /// interaction (TS `AMBIGUOUS_INTERACTION`). It must not count as nil, which would let a
    /// giver lookup fall back to another token.
    Ambiguous,
}

/// Stream lookups bounded by one quest event (TS `InteractionLookup`).
#[derive(Clone, Copy)]
pub struct InteractionLookup<'v, 'a> {
    session: Session<'v, 'a>,
    /// The event's raw time.
    t: f64,
    /// Samples at `t` count up to this precise time: the next event's when it shares `t`.
    before_tp: f64,
}

/// TS `questEventLookup`: the lookup for the event at `index`, or None when the previous or
/// next event has the same `t` and `tp` (its giver could not be told apart).
pub fn quest_event_lookup<'v, 'a>(session: &Session<'v, 'a>, index: usize) -> Option<InteractionLookup<'v, 'a>> {
    let event = session.event(index)?;
    let previous = index.checked_sub(1).and_then(|i| session.event(i));
    let next = session.event(index + 1);
    let indistinguishable = |other: Option<trace_data::Event>| other.is_some_and(|other| other.t() == event.t() && other.tp() == event.tp());
    if indistinguishable(previous) || indistinguishable(next) {
        return None;
    }
    let before_tp = match next {
        Some(next) if next.t() == event.t() => next.tp(),
        _ => f64::INFINITY,
    };
    Some(InteractionLookup { session: *session, t: event.t(), before_tp })
}

impl<'v, 'a> InteractionLookup<'v, 'a> {
    /// `session.functions[name][params...]` as it stood during the interaction.
    pub fn get(&self, name: &str, params: &[KeyRef]) -> InteractionValue<'v, 'a> {
        value_before_next_event(get_stream(&self.session, name, params).unwrap_or(&[]), self.t, self.before_tp)
    }
}

fn entry_tp(entry: &Value) -> f64 {
    entry.get("tp").and_then(Value::as_num).unwrap_or(f64::NAN)
}

fn value_before_next_event<'v, 'a>(stream: &'v [Value<'a>], t: f64, tp: f64) -> InteractionValue<'v, 'a> {
    let (mut lo, mut hi) = (0isize, stream.len() as isize - 1);
    let mut result = None;
    while lo <= hi {
        let mid = ((lo + hi) as usize) >> 1;
        let sample = &stream[mid];
        let sample_t = entry_t(sample);
        if sample_t < t || (sample_t == t && entry_tp(sample) <= tp) {
            result = Some(mid);
            lo = mid as isize + 1;
        } else {
            hi = mid as isize - 1;
        }
    }
    let Some(selected) = result.map(|i| &stream[i]) else { return InteractionValue::Known(None) };
    if entry_t(selected) == t && entry_tp(selected) == tp {
        // Excluding the tied probe and returning the one before it could resurrect stale state.
        return InteractionValue::Ambiguous;
    }
    InteractionValue::Known(entry_v(selected))
}

/// Test fixtures for quest interactions, whose tests need samples and events with a precise
/// time different from `t` (`test/fixtures.ts`).
#[cfg(test)]
pub(crate) mod fixtures {
    use crate::test_support::packed;
    use serde_json::{json, Value as Json};

    /// `apiSample`: a stream entry; None records nil (no `v`).
    pub fn sample(t: f64, tp: f64, v: impl Into<Option<Json>>) -> Json {
        match v.into() {
            Some(v) => json!({ "t": t, "tp": tp, "v": v }),
            None => json!({ "t": t, "tp": tp }),
        }
    }

    /// `traceEvent`: an event with packed args.
    pub fn trace_event(name: &str, t: f64, tp: f64, args: &[Json]) -> Json {
        json!({ "t": t, "tp": tp, "e": name, "a": packed(args) })
    }

    pub fn creature_guid(id: u32) -> Json {
        json!(format!("Creature-0-5208-0-7-{id}-000032"))
    }

    pub fn object_guid(id: u32) -> Json {
        json!(format!("GameObject-0-5208-0-7-{id}-000032"))
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{creature_guid, sample, trace_event};
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::json;

    /// An owned view of a lookup result.
    #[derive(PartialEq, Debug)]
    enum Seen {
        NoLookup,
        Nil,
        Ambiguous,
        Num(f64),
        Str(String),
    }

    fn look_up(session: &serde_json::Value, index: usize, name: &str, params: &[&str]) -> Seen {
        with_session(session, |s| {
            let Some(lookup) = quest_event_lookup(s, index) else { return Seen::NoLookup };
            let params: Vec<KeyRef> = params.iter().map(|&p| KeyRef::Str(p)).collect();
            match lookup.get(name, &params) {
                InteractionValue::Ambiguous => Seen::Ambiguous,
                InteractionValue::Known(None) => Seen::Nil,
                InteractionValue::Known(Some(v)) => match (v.as_num(), v.as_str()) {
                    (Some(n), _) => Seen::Num(n),
                    (_, Some(s)) => Seen::Str(s.to_owned()),
                    _ => panic!("unexpected value {v:?}"),
                },
            }
        })
    }

    fn guid_str(guid: serde_json::Value) -> Seen {
        Seen::Str(guid.as_str().unwrap().to_owned())
    }

    #[test]
    fn has_no_lookup_when_the_previous_event_has_an_indistinguishable_timestamp() {
        let session = session_json(json!({}), vec![trace_event("GOSSIP_SHOW", 3.0, 3.0, &[]), trace_event("GOSSIP_SHOW", 3.0, 3.0, &[])]);
        assert_eq!(look_up(&session, 1, "GetQuestID", &[]), Seen::NoLookup);
    }

    #[test]
    fn has_no_lookup_when_the_next_event_has_an_indistinguishable_timestamp() {
        let session = session_json(json!({}), vec![trace_event("GOSSIP_SHOW", 3.0, 3.0, &[]), trace_event("GOSSIP_SHOW", 3.0, 3.0, &[])]);
        assert_eq!(look_up(&session, 0, "GetQuestID", &[]), Seen::NoLookup);
    }

    #[test]
    fn reads_a_parameterized_stream() {
        let session =
            session_json(json!({ "UnitGUID": { "npc": [sample(3.0, 3.002, creature_guid(276171))] } }), vec![trace_event("GOSSIP_SHOW", 3.0, 3.001, &[])]);
        assert_eq!(look_up(&session, 0, "UnitGUID", &["npc"]), guid_str(creature_guid(276171)));
    }

    #[test]
    fn reads_a_stream_that_was_never_recorded_as_nil() {
        let session = session_json(json!({}), vec![trace_event("GOSSIP_SHOW", 3.0, 3.001, &[])]);
        assert_eq!(look_up(&session, 0, "GetQuestID", &[]), Seen::Nil);
    }

    #[test]
    fn includes_synchronous_probes_recorded_after_the_event_in_the_same_frame() {
        let session = session_json(
            json!({ "GetQuestID": [sample(1.0, 1.0, json!(99196)), sample(3.0, 3.002, json!(99197))] }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.001, &[])],
        );
        assert_eq!(look_up(&session, 0, "GetQuestID", &[]), Seen::Num(99197.0));
    }

    #[test]
    fn excludes_probes_recorded_after_the_next_event_in_the_same_frame() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.002, json!(99196)), sample(3.0, 3.012, json!(99197))] }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.001, &[]), trace_event("MERCHANT_SHOW", 3.0, 3.011, &[])],
        );
        assert_eq!(look_up(&session, 0, "GetQuestID", &[]), Seen::Num(99196.0));
    }

    #[test]
    fn excludes_probes_from_later_frames() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.002, json!(99196)), sample(4.0, 4.002, json!(99197))] }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.001, &[]), trace_event("QUEST_DETAIL", 4.0, 4.001, &[])],
        );
        assert_eq!(look_up(&session, 0, "GetQuestID", &[]), Seen::Num(99196.0));
    }

    #[test]
    fn marks_a_probe_that_ties_with_the_next_event_as_ambiguous() {
        let session = session_json(
            json!({ "GetQuestID": [sample(1.0, 1.0, json!(99196)), sample(3.0, 3.002, json!(99197))] }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.001, &[]), trace_event("QUEST_FINISHED", 3.0, 3.002, &[])],
        );
        assert_eq!(look_up(&session, 0, "GetQuestID", &[]), Seen::Ambiguous);
    }

    #[test]
    fn keeps_a_recorded_nil_instead_of_an_older_value() {
        let session = session_json(
            json!({ "UnitGUID": { "npc": [sample(1.0, 1.0, creature_guid(276171)), sample(2.0, 2.0, None)] } }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.001, &[])],
        );
        assert_eq!(look_up(&session, 0, "UnitGUID", &["npc"]), Seen::Nil);
    }

    #[test]
    fn keeps_an_unchanged_value_from_an_earlier_frame() {
        let session = session_json(
            json!({ "UnitGUID": { "npc": [sample(1.0, 1.0, creature_guid(276171))] } }),
            vec![trace_event("QUEST_TURNED_IN", 60.0, 60.0, &[json!(99196)])],
        );
        assert_eq!(look_up(&session, 0, "UnitGUID", &["npc"]), guid_str(creature_guid(276171)));
    }
}
