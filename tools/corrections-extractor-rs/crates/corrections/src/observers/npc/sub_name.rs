//! npcKeys.subName - Questie field 14. Port of `extract/observers/npc/subName.ts`.
//!
//! `observe` keeps enUS subnames only; `observe_localized` keeps every locale (for the lookup
//! export).
//!
//! Source: UnitSubName.mouseover from Modules/Trackers/UnitState.lua (the line below the name in
//! the unit tooltip). "" means the NPC has no subname. Unlike the other UnitState values, it is
//! not sampled on UPDATE_MOUSEOVER_UNIT but from the tooltip, and only written on change: a value
//! is only paired with the mouseover GUID sampled in the same frame before it, so a tooltip that
//! was not shown for an NPC cannot hand it the previous NPC's subname.
//!
//! NPCs without a subname show their level line there instead ("Level 28", "Stufe ?? (Boss)",
//! "等级 10（精英）"), which the tracker does not tell apart. Such a value means "no subname".
//!
//! Privacy: a player's tooltip shows the guild there. The tracker only records the line for NPC
//! tooltips, and only npc GUID samples are paired with it here.

use crate::aggregate::Num;
use crate::guid::GuidKind;
use crate::js;
use crate::observation::{Confidence, Localized, Observation};
use crate::observers::guid_encounters::guid_encounters;
use crate::probes::locale_at;
use std::collections::HashMap;
use trace_data::stream::{entry_t, entry_tp, entry_v, get_stream};
use trace_data::{KeyRef, Session, Value};

const MOUSEOVER: &str = "mouseover";

pub fn observe_localized(session: &Session) -> Vec<Observation<Localized<String>>> {
    let mouseover = [KeyRef::Str(MOUSEOVER)];
    let sub_names = by_time(get_stream(session, "UnitSubName", &mouseover).unwrap_or_default());
    let guid_samples = by_time(get_stream(session, "UnitGUID", &mouseover).unwrap_or_default());
    let mut observations = Vec::new();
    for encounter in guid_encounters(session, GuidKind::Npc, &[MOUSEOVER]) {
        let frame = Num(encounter.t);
        // A later GUID sample in the same frame starts another unit.
        let next_tp = guid_samples
            .get(&frame)
            .into_iter()
            .flatten()
            .map(|entry| entry_tp(entry))
            .find(|&tp| tp > encounter.tp)
            .unwrap_or(f64::INFINITY);
        let sample = sub_names.get(&frame).into_iter().flatten().find(|entry| {
            let tp = entry_tp(entry);
            tp >= encounter.tp && tp < next_tp
        });
        let Some(sub_name) = sample.and_then(|entry| entry_v(entry)).and_then(Value::as_str) else {
            continue;
        };
        let Some(locale) = locale_at(session, encounter.t) else { continue };
        let value = Localized {
            locale: locale.to_owned(),
            value: if is_level_line(sub_name) { String::new() } else { sub_name.to_owned() },
        };
        observations.push(Observation::new(encounter.entity_id, value, Confidence::High, encounter.t));
    }
    observations
}

pub fn observe(session: &Session) -> Vec<Observation<String>> {
    observe_localized(session)
        .into_iter()
        .filter(|obs| obs.value.locale == "enUS")
        .map(|obs| obs.map(|sub_name| sub_name.value))
        .collect()
}

/// One word, the level (or "??"), and an optional rank such as "(Elite)" in ASCII or full-width
/// brackets: TS `/^\S+ (\d+|\?\?)( ?[(（][^)）]*[)）])?$/u`. Titles with a number elsewhere
/// ("SI:7 Operative", "제2대 칸") do not match.
pub fn is_level_line(sub_name: &str) -> bool {
    // `^\S+ `: the first word ends at the first whitespace, which has to be a plain space.
    let Some(word_end) = sub_name.find(js::is_whitespace).filter(|&end| end > 0) else {
        return false;
    };
    let Some(rest) = sub_name[word_end..].strip_prefix(' ') else { return false };
    // `(\d+|\?\?)`: \d is ASCII only.
    let level_len = if rest.starts_with("??") {
        2
    } else {
        rest.bytes().take_while(u8::is_ascii_digit).count()
    };
    if level_len == 0 {
        return false;
    }
    // `( ?[(（][^)）]*[)）])?$`: nothing, or a bracketed rank whose first closing bracket ends the line.
    let rank = &rest[level_len..];
    if rank.is_empty() {
        return true;
    }
    let Some(inside) = rank.strip_prefix(' ').unwrap_or(rank).strip_prefix(['(', '（']) else {
        return false;
    };
    inside.find([')', '）']).is_some_and(|close| {
        let bracket_len = inside[close..].chars().next().map_or(0, char::len_utf8);
        close + bracket_len == inside.len()
    })
}

/// A stream's entries by `t`, each group in stream order: TS finds the first entry of the whole
/// stream with `entry.t === t`, which is the first of its group. A NaN `t` equals nothing.
fn by_time<'v, 'a>(stream: &'v [Value<'a>]) -> HashMap<Num, Vec<&'v Value<'a>>> {
    let mut groups: HashMap<Num, Vec<&Value>> = HashMap::new();
    for entry in stream {
        let t = entry_t(entry);
        if !t.is_nan() {
            groups.entry(Num(t)).or_default().push(entry);
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{event, session_json, with_session};
    use serde_json::json;

    fn observed(functions: serde_json::Value, events: Vec<serde_json::Value>) -> Vec<(i64, String, Confidence, f64)> {
        with_session(&session_json(functions, events), |s| {
            observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect()
        })
    }

    fn observed_ids(functions: serde_json::Value, events: Vec<serde_json::Value>) -> Vec<i64> {
        observed(functions, events).into_iter().map(|(id, ..)| id).collect()
    }

    #[test]
    fn observes_the_subname_sampled_for_the_mouseover_npc_at_the_same_time() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "UnitGUID": { "mouseover": [{ "t": 3, "tp": 3, "v": "Creature-0-5208-0-7-823-000031" }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Weapon Vendor" }] },
        });
        assert_eq!(observed(functions, vec![]), [(823, "Weapon Vendor".to_owned(), Confidence::High, 3.0)]);
    }

    #[test]
    fn does_not_hand_the_previous_npcs_subname_to_an_npc_whose_tooltip_was_not_sampled() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "UnitGUID": { "mouseover": [
                { "t": 3, "tp": 3, "v": "Creature-0-5208-0-7-823-000031" },
                { "t": 5, "tp": 5, "v": "Creature-0-5208-0-7-824-000032" },
            ]},
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Weapon Vendor" }] },
        });
        let events = vec![event(5.0, "UPDATE_MOUSEOVER_UNIT", json!({ "n": 0 }))];
        assert_eq!(observed_ids(functions, events), [823]);
    }

    #[test]
    fn pairs_a_subname_sampled_in_the_same_frame_only_with_the_last_npc_before_it() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "UnitGUID": { "mouseover": [
                { "t": 3, "tp": 3, "v": "Creature-0-5208-0-7-823-000031" },
                { "t": 3, "tp": 3.2, "v": "Creature-0-5208-0-7-824-000032" },
            ]},
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.3, "v": "Weapon Vendor" }] },
        });
        assert_eq!(observed_ids(functions, vec![]), [824]);
    }

    #[test]
    fn skips_encounters_where_no_subname_was_sampled() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "UnitGUID": { "mouseover": [{ "t": 3, "tp": 3, "v": "Creature-0-5208-0-7-823-000031" }] },
        });
        assert_eq!(observed(functions, vec![]), []);
    }

    #[test]
    fn keeps_the_locale_for_the_lookup_export_and_skips_it_for_the_en_us_export() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "deDE" }] },
            "UnitGUID": { "mouseover": [{ "t": 3, "tp": 3, "v": "Creature-0-5208-0-7-823-000031" }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Waffenhändler" }] },
        });
        let session = session_json(functions, vec![]);
        assert_eq!(with_session(&session, |s| observe(s).len()), 0);
        let localized = with_session(&session, |s| observe_localized(s).into_iter().map(|o| o.value).collect::<Vec<_>>());
        assert_eq!(
            localized,
            [Localized {
                locale: "deDE".to_owned(),
                value: "Waffenhändler".to_owned()
            }]
        );
    }

    #[test]
    fn observes_a_level_line_as_no_subname() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "ruRU" }] },
            "UnitGUID": { "mouseover": [{ "t": 3, "tp": 3, "v": "Creature-0-5208-0-7-823-000031" }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Уровень 11 (элита)" }] },
        });
        let localized = with_session(&session_json(functions, vec![]), |s| {
            observe_localized(s).into_iter().map(|o| o.value).collect::<Vec<_>>()
        });
        assert_eq!(
            localized,
            [Localized {
                locale: "ruRU".to_owned(),
                value: String::new()
            }]
        );
    }

    // describe("isLevelLine")

    #[test]
    fn treats_level_lines_as_level_lines() {
        let level_lines = [
            "Level 28",
            "Level 10 (Elite)",
            "Level ??",
            "Level ?? (Boss)",
            "Stufe 11 (Elite)",
            "Niveau ?? (Élite)",
            "Nivel ?? (Jefe)",
            "Nível 11 (Elite)",
            "Уровень 18",
            "레벨 11 (정예)",
            "等级 10（精英）",
            "等級 11（精英）",
            "<name> 10",
            "Level 11 (<name>)",
        ];
        for sub_name in level_lines {
            assert!(is_level_line(sub_name), "{sub_name} is a level line");
        }
    }

    #[test]
    fn keeps_titles_as_subnames() {
        let titles = [
            "Weapon Vendor",
            "Waffenhändler",
            "SI:7 Operative",
            "Leader of SI:7",
            "Agent du SI:7",
            "제2대 칸",
            "Level Trainer",
            "",
        ];
        for sub_name in titles {
            assert!(!is_level_line(sub_name), "{sub_name:?} is a subname");
        }
    }

    // Not in the TS tests: where the regex stops matching, read the way JS reads it.

    #[test]
    fn matches_the_level_line_pattern_exactly() {
        let not_level_lines = [
            " 10",                // \S+ needs a first word
            "Level  10",          // one plain space, not two
            "Level\u{a0}10",      // a no-break space is whitespace, but not the space
            "Level ???",          // exactly two question marks
            "Level ١٠",           // \d is ASCII only
            "Level 10 (Elite) x", // the rank ends the line
            "Level 10 (Elite))",  // the first closing bracket ends the line
            "Level 10  (Elite)",  // at most one space before the rank
            "Level 10 Elite",     // the rank is bracketed
            "Level 10 (Elite",    // and closed
        ];
        for sub_name in not_level_lines {
            assert!(!is_level_line(sub_name), "{sub_name:?} is no level line");
        }
        // Mixed brackets and an empty rank match [(（][^)）]*[)）].
        for sub_name in ["Level 10 (Elite）", "Level 10 ()", "Level 10(Elite)"] {
            assert!(is_level_line(sub_name), "{sub_name:?} is a level line");
        }
    }

    // Not in the TS tests: the privacy rule of CLAUDE.md for tooltip lines.

    #[test]
    fn never_hands_a_line_sampled_after_a_players_guid_to_the_npc_before_it() {
        // A player's tooltip line is their guild. Should one be recorded, the player's GUID
        // sample in the same frame ends the npc's pairing window before it.
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "UnitGUID": { "mouseover": [
                { "t": 3, "tp": 3, "v": "Creature-0-5208-0-7-823-000031" },
                { "t": 3, "tp": 3.2, "v": "Player-5284-0362ABCD" },
            ]},
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.3, "v": "Some Guild" }] },
        });
        assert_eq!(observed(functions, vec![]), []);
    }
}
