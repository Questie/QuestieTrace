//! questKeys.objectivesText - Questie field 8, a list of lines. Port of
//! `extract/observers/quest/objectivesText.ts`.
//!
//! Questie shows this as the quest's objective text in the tooltip (`QO.Description =
//! QO.objectivesText`), so it is the objectives summary, not the flavour text. The client
//! source is the second return of GetQuestLogQuestText(questLogIndex) -> questDescription,
//! questObjectives, recorded as one packed tuple under the questID. GetObjectiveText carries
//! the same text, but in a flat stream attributed only through GetQuestID(), so it is not used.
//!
//! Questie stores the text split into lines with the blank lines between paragraphs kept (a
//! two-paragraph summary is `{line1, "", line2}`).

use crate::js;
use crate::observation::{Confidence, Localized, Observation};
use crate::probes::locale_at;
use trace_data::stream::emulated_list;
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

/// Questie's objectivesText: the summary split into lines. JS shape: an array of strings.
pub type QuestObjectivesTextValue = Vec<String>;

/// Position of `questObjectives` in the packed GetQuestLogQuestText tuple.
const OBJECTIVES_TUPLE_INDEX: usize = 1;

/// The summary's lines, each trimmed. Interior blank lines are kept; leading and trailing ones
/// dropped, so a summary that is blank produces no lines (auto-complete quests have none).
fn split_lines(value: &str) -> Vec<String> {
    // `split(/\r?\n/)`: a '\r' left before a '\n' is trimmed with the line anyway.
    let lines: Vec<&str> = value.split('\n').map(js::trim).collect();
    let first = lines.iter().position(|line| !line.is_empty());
    let last = lines.iter().rposition(|line| !line.is_empty());
    match (first, last) {
        (Some(first), Some(last)) => lines[first..=last].iter().map(|line| (*line).to_owned()).collect(),
        _ => Vec::new(),
    }
}

fn read_objectives_text<'v>(value: Option<&'v Value>) -> Option<&'v str> {
    emulated_list(value)?.get(OBJECTIVES_TUPLE_INDEX).copied().flatten()?.as_str()
}

pub fn observe_localized(session: &Session) -> Vec<Observation<Localized<QuestObjectivesTextValue>>> {
    let mut observations = Vec::new();
    let Some(root) = session.function("GetQuestLogQuestText").filter(|root| !root.is_array()) else {
        return observations;
    };

    for quest_id_key in param_keys(root) {
        let quest_id = js::number(&quest_id_key);
        if !quest_id.is_finite() {
            continue;
        }
        for entry in get_stream(session, "GetQuestLogQuestText", &[KeyRef::Str(&quest_id_key)]).unwrap_or(&[]) {
            let t = entry_t(entry);
            // Older traces may not contain GetLocale: no locale is the legacy enUS default.
            let locale = locale_at(session, t).unwrap_or("enUS");
            let Some(text) = read_objectives_text(entry_v(entry)) else { continue };
            let lines = split_lines(text);
            if lines.is_empty() {
                continue;
            }
            let value = Localized {
                locale: locale.to_owned(),
                value: lines,
            };
            // Read straight out of the quest log API.
            observations.push(Observation::new(quest_id as i64, value, Confidence::High, t));
        }
    }
    observations
}

pub fn observe(session: &Session) -> Vec<Observation<QuestObjectivesTextValue>> {
    observe_localized(session)
        .into_iter()
        .filter(|obs| obs.value.locale == "enUS")
        .map(|obs| obs.map(|v| v.value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{packed, session_json, stream, with_session};
    use serde_json::{json, Value as Json};

    /// Packed GetQuestLogQuestText return: (questDescription, questObjectives).
    fn quest_text(description: &str, objectives: &str) -> Json {
        packed(&[json!(description), json!(objectives)])
    }

    fn observed(functions: Json) -> Vec<Observation<QuestObjectivesTextValue>> {
        with_session(&session_json(functions, vec![]), observe)
    }

    fn lines(observation: &Observation<QuestObjectivesTextValue>) -> Vec<&str> {
        observation.value.iter().map(String::as_str).collect()
    }

    #[test]
    fn takes_the_objectives_return_not_the_description() {
        let observations = observed(json!({
            "GetQuestLogQuestText": { "33": stream([(10.0, quest_text("Eagan has a hunting problem.", "Bring 8 pieces of Tough Wolf Meat to Eagan Peltskinner outside Northshire Abbey."))]) },
        }));
        let observed: Vec<_> = observations.into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect();
        let expected = vec!["Bring 8 pieces of Tough Wolf Meat to Eagan Peltskinner outside Northshire Abbey.".to_owned()];
        assert_eq!(observed, [(33, expected, Confidence::High, 10.0)]);
    }

    #[test]
    fn keeps_the_blank_line_between_paragraphs() {
        let observations = observed(json!({
            "GetQuestLogQuestText": { "25": stream([(10.0, quest_text("Long flavour text.", "Dispatch 12 Befouled Water Elementals at Mystral Lake.\n\nScout the gazebo on Mystral Lake.\n\nReturn to Mastok Wrilehiss at Splintertree Post."))]) },
        }));
        assert_eq!(
            lines(&observations[0]),
            [
                "Dispatch 12 Befouled Water Elementals at Mystral Lake.",
                "",
                "Scout the gazebo on Mystral Lake.",
                "",
                "Return to Mastok Wrilehiss at Splintertree Post."
            ]
        );
    }

    #[test]
    fn drops_leading_and_trailing_blank_lines_but_keeps_interior_ones() {
        let observations = observed(json!({
            "GetQuestLogQuestText": { "7": stream([(10.0, quest_text("d", "\nKill 10 Kobold Vermin, then return to Marshal McBride.\n\n"))]) },
        }));
        assert_eq!(lines(&observations[0]), ["Kill 10 Kobold Vermin, then return to Marshal McBride."]);
    }

    #[test]
    fn skips_empty_objectives_text() {
        let observations = observed(json!({ "GetQuestLogQuestText": { "9": stream([(10.0, quest_text("d", "   \n  "))]) } }));
        assert_eq!(observations, []);
    }

    #[test]
    fn skips_non_en_us_sessions() {
        let observations = observed(json!({
            "GetLocale": { "player": stream([(0.0, json!("deDE"))]) },
            "GetQuestLogQuestText": { "33": stream([(10.0, quest_text("d", "Bring 8 pieces of Tough Wolf Meat."))]) },
        }));
        assert_eq!(observations, []);
    }

    #[test]
    fn treats_a_missing_get_locale_stream_as_en_us() {
        let observations = observed(json!({ "GetQuestLogQuestText": { "33": stream([(10.0, quest_text("d", "Bring 8 pieces of Tough Wolf Meat."))]) } }));
        assert_eq!(observations.len(), 1);
    }

    #[test]
    fn emits_every_sample_including_repeats() {
        let observations = observed(json!({
            "GetQuestLogQuestText": { "33": stream([
                (10.0, quest_text("d", "Bring 8 pieces of Tough Wolf Meat.")),
                (40.0, quest_text("d", "Bring 8 pieces of Tough Wolf Meat.")),
            ]) },
        }));
        assert_eq!(observations.iter().map(|o| o.t).collect::<Vec<_>>(), [10.0, 40.0]);
    }

    #[test]
    fn observes_nothing_without_a_get_quest_log_quest_text_stream() {
        assert_eq!(observed(json!({})), []);
    }

    #[test]
    fn splits_crlf_lines_like_the_ts_regex() {
        assert_eq!(split_lines("a\r\n\r\nb\r\n"), ["a", "", "b"]);
    }
}
