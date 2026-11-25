//! npcKeys.subName - Questie field 14. Port of `extract/observers/npc/subName.ts`.
//!
//! `observe` keeps enUS subnames only; `observe_localized` keeps every locale (for the lookup
//! export).
//!
//! Source: UnitSubName.mouseover from Modules/Trackers/UnitState.lua (the line below the name in
//! the unit tooltip), written on change. "" means the NPC has no subname.
//!
//! NPCs without a subname show their level line there instead ("Level 28", "Stufe ?? (Boss)",
//! "等级 10（精英）"), which older versions of the tracker did not tell apart. Such a value means "no
//! subname".
//!
//! Which NPC a subname belongs to depends on the tracker version:
//! - With UnitSubNameGUID.mouseover (schema 21+), every subname has the GUID of the NPC its
//!   tooltip was shown for, so it is used as recorded.
//! - Without it, the NPC is inferred. The mouseover GUID stream cannot be used: UPDATE_MOUSEOVER_UNIT
//!   was not registered, so mouseovers were only sampled as a side effect of other events and a
//!   title was often paired with a neighbouring NPC. Instead, players hover an NPC before they
//!   target it with the mouse, so the last subname before PLAYER_TARGET_CHANGED to an NPC is that
//!   NPC's. It is only trusted when it is recent and the player is not in combat (target changes in
//!   combat are mostly tab targeting). Compared with the Classic NPC database, this was right for
//!   99% of the titled NPCs on a sample of 848 recent sessions. Targeting without hovering still
//!   pairs a wrong title now and then.
//!
//! Privacy: a player's tooltip shows the guild there. The tracker only records the line for NPC
//! tooltips, and only npc GUIDs are paired with it here.

use crate::aggregate::Num;
use crate::guid::{parse_guid, GuidKind};
use crate::js;
use crate::observation::{Confidence, Localized, Observation};
use crate::observers::guid_encounters::guid_encounters;
use crate::probes::locale_at;
use std::collections::HashMap;
use trace_data::stream::{entry_t, entry_tp, entry_v, get_stream};
use trace_data::{KeyRef, Session, Value};

const MOUSEOVER: &str = "mouseover";

/// Longest time (seconds) between a subname and the target change it is inferred for.
const MAX_SECONDS_BEFORE_TARGET: f64 = 1.0;

pub fn observe_localized(session: &Session) -> Vec<Observation<Localized<String>>> {
    let mouseover = [KeyRef::Str(MOUSEOVER)];
    let sub_names = get_stream(session, "UnitSubName", &mouseover).unwrap_or_default();
    match get_stream(session, "UnitSubNameGUID", &mouseover) {
        Some(guids) => observe_recorded_guids(session, sub_names, guids),
        None => observe_before_target_change(session, sub_names),
    }
}

fn observe_recorded_guids(session: &Session, sub_names: &[Value], guids: &[Value]) -> Vec<Observation<Localized<String>>> {
    let mut guid_at: HashMap<(Num, Num), Option<&str>> = HashMap::new();
    for entry in guids {
        guid_at.insert((Num(entry_t(entry)), Num(entry_tp(entry))), entry_v(entry).and_then(Value::as_str));
    }
    sub_names
        .iter()
        .filter_map(|sample| {
            let at = (Num(entry_t(sample)), Num(entry_tp(sample)));
            let parsed = parse_guid(guid_at.get(&at).copied().flatten());
            let id = parsed.id.filter(|_| parsed.kind == GuidKind::Npc)?;
            observation(session, id as i64, sample, entry_t(sample))
        })
        .collect()
}

fn observe_before_target_change(session: &Session, sub_names: &[Value]) -> Vec<Observation<Localized<String>>> {
    let mut samples: Vec<&Value> = sub_names.iter().collect();
    samples.sort_by(|a, b| entry_tp(a).total_cmp(&entry_tp(b)));
    let combat = combat_changes(session);
    let mut observations = Vec::new();
    for encounter in guid_encounters(session, GuidKind::Npc, &["target"]) {
        let before = samples.partition_point(|sample| entry_tp(sample) <= encounter.tp);
        let Some(sample) = before.checked_sub(1).map(|at| samples[at]) else {
            continue;
        };
        if encounter.tp - entry_tp(sample) > MAX_SECONDS_BEFORE_TARGET {
            continue;
        }
        let in_combat = combat
            .iter()
            .take_while(|change| change.tp <= encounter.tp)
            .last()
            .is_some_and(|change| change.in_combat);
        if in_combat {
            continue;
        }
        observations.extend(observation(session, encounter.entity_id, sample, encounter.t));
    }
    observations
}

/// The subname of `sample`, observed at `t`, with the locale it was sampled in.
fn observation(session: &Session, entity_id: i64, sample: &Value, t: f64) -> Option<Observation<Localized<String>>> {
    let sub_name = entry_v(sample).and_then(Value::as_str)?;
    let locale = locale_at(session, entry_t(sample))?;
    let value = Localized {
        locale: locale.to_owned(),
        value: if is_level_line(sub_name) { String::new() } else { sub_name.to_owned() },
    };
    Some(Observation::new(entity_id, value, Confidence::High, t))
}

struct CombatChange {
    tp: f64,
    in_combat: bool,
}

/// When (tp) the player entered or left combat, in event order.
fn combat_changes(session: &Session) -> Vec<CombatChange> {
    session
        .events()
        .filter(|event| event.is("PLAYER_REGEN_DISABLED") || event.is("PLAYER_REGEN_ENABLED"))
        .map(|event| CombatChange {
            tp: event.tp(),
            in_combat: event.is("PLAYER_REGEN_DISABLED"),
        })
        .collect()
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

    fn localized(functions: serde_json::Value) -> Vec<Localized<String>> {
        with_session(&session_json(functions, vec![]), |s| {
            observe_localized(s).into_iter().map(|o| o.value).collect()
        })
    }

    const NORDUN: &str = "Creature-0-5208-0-7-268238-000031";
    const ISAAC: &str = "Creature-0-5208-0-7-258930-000032";
    const CHEST: &str = "GameObject-0-5208-0-7-2843-0000399";

    /// The `stream` functions with an enUS client.
    fn en_us(functions: serde_json::Value) -> serde_json::Value {
        let mut functions = functions;
        functions["GetLocale"] = json!({ "player": [{ "t": 0, "tp": 0, "v": "enUS" }] });
        functions
    }

    fn no_args() -> serde_json::Value {
        json!({ "n": 0 })
    }

    // describe("observeSubName (npc) with recorded guids")

    #[test]
    fn observes_the_subname_for_the_guid_recorded_with_it() {
        let functions = en_us(json!({
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Weapon Vendor" }] },
            "UnitSubNameGUID": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Creature-0-5208-0-7-823-000031" }] },
        }));
        assert_eq!(observed(functions, vec![]), [(823, "Weapon Vendor".to_owned(), Confidence::High, 3.0)]);
    }

    #[test]
    fn does_not_hand_a_neighbours_subname_to_the_npc_that_was_sampled_at_the_same_time() {
        let functions = en_us(json!({
            "UnitGUID": { "mouseover": [{ "t": 3, "tp": 3, "v": NORDUN }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Isaac Chan's Companion" }] },
            "UnitSubNameGUID": { "mouseover": [{ "t": 3, "tp": 3.1, "v": ISAAC }] },
        }));
        assert_eq!(observed_ids(functions, vec![]), [258930]);
    }

    #[test]
    fn observes_every_npc_that_had_the_same_subname() {
        let functions = en_us(json!({
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Innkeeper" }, { "t": 5, "tp": 5.1, "v": "Innkeeper" }] },
            "UnitSubNameGUID": { "mouseover": [
                { "t": 3, "tp": 3.1, "v": "Creature-0-5208-0-7-823-000031" },
                { "t": 5, "tp": 5.1, "v": "Creature-0-5208-0-7-824-000032" },
            ]},
        }));
        assert_eq!(observed_ids(functions, vec![]), [823, 824]);
    }

    #[test]
    fn skips_subnames_without_an_npc_guid() {
        let functions = en_us(json!({
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Innkeeper" }, { "t": 4, "tp": 4.1, "v": "Innkeeper" }] },
            "UnitSubNameGUID": { "mouseover": [{ "t": 3, "tp": 3.1, "v": CHEST }] },
        }));
        assert_eq!(observed(functions, vec![]), []);
    }

    #[test]
    fn keeps_the_locale_for_the_lookup_export_and_skips_it_for_the_en_us_export() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "deDE" }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Waffenhändler" }] },
            "UnitSubNameGUID": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Creature-0-5208-0-7-823-000031" }] },
        });
        assert_eq!(observed(functions.clone(), vec![]), []);
        let waffenhaendler = Localized {
            locale: "deDE".to_owned(),
            value: "Waffenhändler".to_owned(),
        };
        assert_eq!(localized(functions), [waffenhaendler]);
    }

    #[test]
    fn observes_a_level_line_as_no_subname() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "ruRU" }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Уровень 11 (элита)" }] },
            "UnitSubNameGUID": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Creature-0-5208-0-7-823-000031" }] },
        });
        let none = Localized {
            locale: "ruRU".to_owned(),
            value: String::new(),
        };
        assert_eq!(localized(functions), [none]);
    }

    // describe("observeSubName (npc) without recorded guids")

    #[test]
    fn observes_the_last_subname_before_the_target_changes_to_an_npc() {
        let functions = en_us(json!({
            "UnitGUID": { "target": [{ "t": 3, "tp": 3.5, "v": NORDUN }] },
            "UnitSubName": { "mouseover": [
                { "t": 1, "tp": 1, "v": "Isaac Chan's Companion" },
                { "t": 3, "tp": 3.1, "v": "Traveling Ranged Weapon Salesman" },
            ]},
        }));
        let salesman = (268238, "Traveling Ranged Weapon Salesman".to_owned(), Confidence::High, 3.0);
        assert_eq!(observed(functions, vec![]), [salesman]);
    }

    #[test]
    fn does_not_pair_a_subname_recorded_more_than_a_second_before_the_target_change() {
        let functions = en_us(json!({
            "UnitGUID": { "target": [{ "t": 4.2, "tp": 4.2, "v": NORDUN }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Traveling Ranged Weapon Salesman" }] },
        }));
        assert_eq!(observed(functions, vec![]), []);
    }

    #[test]
    fn does_not_pair_a_subname_recorded_after_the_target_change() {
        let functions = en_us(json!({
            "UnitGUID": { "target": [{ "t": 3, "tp": 3, "v": NORDUN }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Isaac Chan's Companion" }] },
        }));
        assert_eq!(observed(functions, vec![]), []);
    }

    /// Nordun is targeted at 3.5 and Isaac at 6.5, each right after hovering.
    fn two_targets() -> serde_json::Value {
        en_us(json!({
            "UnitGUID": { "target": [{ "t": 3.5, "tp": 3.5, "v": NORDUN }, { "t": 6.5, "tp": 6.5, "v": ISAAC }] },
            "UnitSubName": { "mouseover": [
                { "t": 3, "tp": 3.1, "v": "Traveling Ranged Weapon Salesman" },
                { "t": 6.2, "tp": 6.2, "v": "Isaac Chan's Companion" },
            ]},
        }))
    }

    #[test]
    fn does_not_pair_target_changes_in_combat() {
        let events = vec![event(2.0, "PLAYER_REGEN_DISABLED", no_args()), event(6.0, "PLAYER_REGEN_ENABLED", no_args())];
        assert_eq!(observed_ids(two_targets(), events), [258930]);
    }

    #[test]
    fn does_not_pair_targets_that_are_not_npcs_or_are_cleared() {
        let functions = en_us(json!({
            "UnitGUID": { "target": [{ "t": 3.5, "tp": 3.5, "v": CHEST }, { "t": 3.6, "tp": 3.6 }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Innkeeper" }] },
        }));
        assert_eq!(observed(functions, vec![]), []);
    }

    #[test]
    fn observes_a_level_line_before_the_target_change_as_no_subname() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "deDE" }] },
            "UnitGUID": { "target": [{ "t": 3.5, "tp": 3.5, "v": NORDUN }] },
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.1, "v": "Stufe ?? (Boss)" }] },
        });
        let none = Localized {
            locale: "deDE".to_owned(),
            value: String::new(),
        };
        assert_eq!(localized(functions), [none]);
    }

    #[test]
    fn skips_sessions_without_a_subname_stream() {
        let functions = en_us(json!({ "UnitGUID": { "target": [{ "t": 3, "tp": 3, "v": NORDUN }] } }));
        assert_eq!(observed(functions, vec![]), []);
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
    fn never_hands_a_line_sampled_after_a_player_to_the_npc_before_it() {
        // A player's tooltip line is their guild. The tracker never records it; should it, the
        // player's sample in the same frame (UnitInteraction.lua records a player as a sample
        // without a GUID) still ends the npc's pairing window before it.
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "UnitGUID": { "mouseover": [
                { "t": 3, "tp": 3, "v": "Creature-0-5208-0-7-823-000031" },
                { "t": 3, "tp": 3.2 },
            ]},
            "UnitSubName": { "mouseover": [{ "t": 3, "tp": 3.3, "v": "Some Guild" }] },
        });
        assert_eq!(observed(functions, vec![]), []);
    }
}
