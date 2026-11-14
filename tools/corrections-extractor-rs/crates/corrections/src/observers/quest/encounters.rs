//! Port of `extract/observers/quest/_encounters.ts`: when a session saw a quest, as anchors that
//! quest observers probe other streams at. Not a Questie field itself.
//!
//! GetQuestID() is sampled with GetTitleText()/GetQuestText() every time a quest frame opens,
//! updates or closes (Modules/Trackers/QuestDialog.lua). Unlike NPCs and objects, the quest id
//! is the stream's own value, so no GUID parsing is needed.

use trace_data::stream::{entry_t, entry_v, get_stream};
use trace_data::{Session, Value};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct QuestEncounter {
    /// Any nonzero number GetQuestID() returned (TS keeps it a JS number, so NaN passes too).
    pub quest_id: f64,
    pub t: f64,
}

/// Every sample in which GetQuestID() returned a nonzero number (an active quest frame). The
/// same quest can appear many times; merging them is the field merges' job.
pub fn quest_encounters(session: &Session) -> Vec<QuestEncounter> {
    let Some(stream) = get_stream(session, "GetQuestID", &[]) else {
        return Vec::new();
    };
    stream
        .iter()
        .filter_map(|entry| {
            let quest_id = entry_v(entry).and_then(Value::as_num).filter(|&id| id != 0.0)?;
            Some(QuestEncounter { quest_id, t: entry_t(entry) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, stream, with_session};
    use serde_json::json;

    fn encounters(functions: serde_json::Value) -> Vec<(f64, f64)> {
        with_session(&session_json(functions, vec![]), |s| {
            quest_encounters(s).into_iter().map(|e| (e.quest_id, e.t)).collect()
        })
    }

    #[test]
    fn emits_one_encounter_per_nonzero_get_quest_id_sample() {
        let functions = json!({ "GetQuestID": stream([(0.0, json!(96659)), (5.0, json!(12345))]) });
        assert_eq!(encounters(functions), [(96659.0, 0.0), (12345.0, 5.0)]);
    }

    #[test]
    fn ignores_zero_which_means_no_active_quest_frame() {
        let functions = json!({ "GetQuestID": stream([(0.0, json!(96659)), (5.0, json!(0))]) });
        assert_eq!(encounters(functions), [(96659.0, 0.0)]);
    }

    #[test]
    fn returns_nothing_without_a_get_quest_id_stream() {
        assert_eq!(encounters(json!({})), []);
    }
}
