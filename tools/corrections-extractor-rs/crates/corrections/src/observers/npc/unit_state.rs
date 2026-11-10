//! Pairs an NPC encounter on the "target"/"mouseover" token with a value that
//! Modules/Trackers/UnitState.lua sampled for that token (UnitLevel, UnitReaction, ...).
//! Port of `extract/observers/npc/_unitState.ts`.
//!
//! Both the UnitGUID and the UnitState streams are only written on change, so a value is only
//! trusted at a GUID sample when it was recorded at the same time, or when
//! PLAYER_TARGET_CHANGED/UPDATE_MOUSEOVER_UNIT fired at that time (UnitState samples every
//! token on both, so the last value is still current). Other GUID samples (quest, merchant and
//! loot events) are not paired. A failed API call at such an event would leave the previous
//! unit's value in place; the field merges have to guard against that.

use crate::aggregate::Num;
use crate::guid::GuidKind;
use crate::observers::guid_encounters::guid_encounters;
use rustc_hash::{FxHashMap, FxHashSet};
use trace_data::stream::{entry_t, entry_tp, entry_v, get_stream, last_at_or_before, value_at};
use trace_data::{KeyRef, Session, Value};

const UNIT_STATE_TOKENS: [&str; 2] = ["target", "mouseover"];
const UNIT_STATE_SAMPLE_EVENTS: [&str; 2] = ["PLAYER_TARGET_CHANGED", "UPDATE_MOUSEOVER_UNIT"];

#[derive(Clone, Copy, Debug)]
pub struct UnitStateEncounter<'v, 'a> {
    pub npc_id: i64,
    pub token: &'static str,
    pub t: f64,
    pub value: &'v Value<'a>,
}

/// Every NPC encounter with its paired UnitState value, token by token. The stream is
/// `name` with `leading_params` before the unit token, e.g. ("UnitLevel", []) or
/// ("UnitReaction", ["player"]).
pub fn unit_state_encounters<'v, 'a>(session: &Session<'v, 'a>, name: &str, leading_params: &[&str]) -> Vec<UnitStateEncounter<'v, 'a>> {
    let mut out = Vec::new();
    let sample_event_times: Vec<f64> = session
        .events()
        .filter(|event| event.name().is_some_and(|e| UNIT_STATE_SAMPLE_EVENTS.contains(&e)))
        .map(|event| event.t())
        .collect();
    for token in UNIT_STATE_TOKENS {
        let params: Vec<KeyRef> = leading_params.iter().map(|&param| KeyRef::Str(param)).chain([KeyRef::Str(token)]).collect();
        let stream = get_stream(session, name, &params).unwrap_or(&[]);
        // JS Map keyed by number: NaN matches NaN and -0 matches 0, as with Num.
        let mut guid_tps: FxHashMap<Num, Vec<f64>> = FxHashMap::default();
        for entry in get_stream(session, "UnitGUID", &[KeyRef::Str(token)]).unwrap_or(&[]) {
            guid_tps.entry(Num(entry_t(entry))).or_default().push(entry_tp(entry));
        }
        let sample_times: FxHashSet<Num> = sample_event_times.iter().copied().chain(stream.iter().map(entry_t)).map(Num).collect();
        for encounter in guid_encounters(session, GuidKind::Npc, &[token]) {
            if !sample_times.contains(&Num(encounter.t)) {
                continue;
            }
            // A later GUID sample in the same frame starts another unit, so its value must not be used.
            // Relies on UnitInteraction.lua sampling the GUID before UnitState.lua samples the value (.toc order).
            let next_tp = guid_tps
                .get(&Num(encounter.t))
                .and_then(|tps| tps.iter().copied().find(|&tp| tp > encounter.tp));
            let value = match next_tp {
                None => value_at(stream, encounter.t),
                Some(next_tp) => {
                    let before_next: Vec<&Value> = stream
                        .iter()
                        .filter(|entry| entry_t(entry) < encounter.t || entry_tp(entry) < next_tp)
                        .collect();
                    // emulator.ts valueAt over the filtered stream.
                    last_at_or_before(before_next.len(), |i| entry_t(before_next[i]) <= encounter.t).and_then(|i| entry_v(before_next[i]))
                }
            };
            let Some(value) = value else { continue };
            out.push(UnitStateEncounter {
                npc_id: encounter.entity_id,
                token,
                t: encounter.t,
                value,
            });
        }
    }
    out
}
