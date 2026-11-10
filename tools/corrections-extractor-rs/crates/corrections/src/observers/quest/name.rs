//! questKeys.name - Questie field 1. Port of `extract/observers/quest/name.ts`.
//!
//! [`observe`] keeps enUS names only; [`observe_localized`] keeps every locale (for the lookup
//! export).

use crate::aggregate::Num;
use crate::js;
use crate::observation::{Confidence, Localized, Observation};
use crate::probes::{locale_at, quest_info_title_at, quest_title_at};
use std::collections::HashSet;
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session};

const QUEST_INFO: &str = "C_QuestLog.GetInfo";

/// _encounters.ts `questEncounters`: (questID, t) of every nonzero `GetQuestID()` sample.
///
/// A private copy: `observers/quest/encounters.rs` (group A) ports the same helper.
fn quest_encounters(session: &Session) -> Vec<(f64, f64)> {
    let Some(stream) = get_stream(session, "GetQuestID", &[]) else { return Vec::new() };
    stream
        .iter()
        // `typeof v === "number" && v !== 0`: NaN passes.
        .filter_map(|entry| entry_v(entry)?.as_num().filter(|&id| id != 0.0).map(|id| (id, entry_t(entry))))
        .collect()
}

pub fn observe_localized(session: &Session) -> Vec<Observation<Localized<String>>> {
    let mut observations = Vec::new();
    let mut quest_ids_with_dialog_title: HashSet<Num> = HashSet::new();
    let localized = |locale: &str, title: &str| Localized { locale: locale.to_owned(), value: title.to_owned() };

    // Quest-dialog titles are the most direct source and keep precedence over the quest-log
    // API when both are available for the same quest.
    for (quest_id, t) in quest_encounters(session) {
        let Some(locale) = locale_at(session, t) else { continue };
        if let Some(title) = quest_title_at(session, t) {
            quest_ids_with_dialog_title.insert(Num(quest_id));
            observations.push(Observation::new(quest_id as i64, localized(locale, title), Confidence::High, t));
        }
    }

    // C_QuestLog.GetInfo is keyed by questID and also recorded for quests in the log that never
    // open a quest dialog, so it is a source in its own right, not only a fallback.
    let Some(info_root) = session.function(QUEST_INFO).filter(|root| !root.is_array()) else { return observations };
    for quest_id_key in param_keys(info_root) {
        let quest_id = js::number(&quest_id_key);
        if !quest_id.is_finite() || quest_ids_with_dialog_title.contains(&Num(quest_id)) {
            continue;
        }
        for entry in get_stream(session, QUEST_INFO, &[KeyRef::Str(&quest_id_key)]).unwrap_or(&[]) {
            let t = entry_t(entry);
            let Some(locale) = locale_at(session, t) else { continue };
            if let Some(title) = quest_info_title_at(session, quest_id, t) {
                observations.push(Observation::new(quest_id as i64, localized(locale, title), Confidence::High, t));
            }
        }
    }

    observations
}

pub fn observe(session: &Session) -> Vec<Observation<String>> {
    observe_localized(session).into_iter().filter(|obs| obs.value.locale == "enUS").map(|obs| obs.map(|v| v.value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, stream, with_session};
    use serde_json::json;

    fn observed(functions: serde_json::Value) -> Vec<(i64, String, Confidence, f64)> {
        with_session(&session_json(functions, vec![]), |s| observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect())
    }

    fn high(quest_id: i64, title: &str, t: f64) -> (i64, String, Confidence, f64) {
        (quest_id, title.to_owned(), Confidence::High, t)
    }

    #[test]
    fn observes_the_quest_title_at_the_same_time_as_the_quest_id_encounter() {
        let functions = json!({
            "GetLocale": { "player": stream([(0.0, json!("enUS"))]) },
            "GetQuestID": stream([(3.0, json!(96659))]),
            "GetTitleText": stream([(3.0, json!("A Threat Within"))]),
        });
        assert_eq!(observed(functions), [high(96659, "A Threat Within", 3.0)]);
    }

    #[test]
    fn uses_the_most_recent_title_at_or_before_the_encounter() {
        let functions = json!({
            "GetLocale": { "player": stream([(0.0, json!("enUS"))]) },
            "GetQuestID": stream([(5.0, json!(96659))]),
            "GetTitleText": stream([(2.0, json!("A Threat Within"))]),
        });
        assert_eq!(observed(functions), [high(96659, "A Threat Within", 5.0)]);
    }

    #[test]
    fn falls_back_to_the_quest_title_from_c_quest_log_get_info() {
        let functions = json!({
            "GetLocale": { "player": stream([(0.0, json!("enUS"))]) },
            "GetQuestID": stream([(3.0, json!(98013))]),
            "C_QuestLog.GetInfo": { "98013": stream([(2.0, json!({ "title": "Swelling Forces", "level": 80 }))]) },
        });
        assert_eq!(observed(functions), [high(98013, "Swelling Forces", 2.0)]);
    }

    #[test]
    fn observes_get_info_titles_without_a_quest_dialog_encounter() {
        let functions = json!({
            "GetLocale": { "player": stream([(0.0, json!("enUS"))]) },
            "C_QuestLog.GetInfo": { "98013": stream([(2.0, json!({ "title": "Swelling Forces" }))]) },
        });
        assert_eq!(observed(functions), [high(98013, "Swelling Forces", 2.0)]);
    }

    #[test]
    fn prefers_get_title_text_over_get_info_when_both_are_present() {
        let functions = json!({
            "GetLocale": { "player": stream([(0.0, json!("enUS"))]) },
            "GetQuestID": stream([(3.0, json!(98013))]),
            "GetTitleText": stream([(3.0, json!("Quest Dialog Title"))]),
            "C_QuestLog.GetInfo": { "98013": stream([(2.0, json!({ "title": "Swelling Forces" }))]) },
        });
        assert_eq!(observed(functions), [high(98013, "Quest Dialog Title", 3.0)]);
    }

    #[test]
    fn skips_encounters_without_a_title() {
        let functions = json!({
            "GetLocale": { "player": stream([(0.0, json!("enUS"))]) },
            "GetQuestID": stream([(3.0, json!(96659))]),
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn skips_encounters_with_a_non_en_us_locale() {
        let functions = json!({
            "GetLocale": { "player": stream([(0.0, json!("frFR"))]) },
            "GetQuestID": stream([(3.0, json!(96659))]),
            "GetTitleText": stream([(3.0, json!("Une menace intérieure"))]),
        });
        assert_eq!(observed(functions), []);
    }
}
