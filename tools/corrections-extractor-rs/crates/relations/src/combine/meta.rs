//! relations-meta.json: the summary of a combine run that ships next to the Lua module. The trace
//! analyzer shows it with the module, and it marks a complete run the way the field extractor's
//! meta.json does: combine removes it first and writes it last. It has its own name so the two
//! pipelines, which share output/, never overwrite each other's summary.

use super::edges::{Domain, Reference};
use super::lua::{LuaRecords, Written};
use super::views::{QuestView, Status};
use crate::model::{CandidateFile, QuestEpisode};
use crate::signals::forever_episodes;
use rustc_hash::FxHashSet;
use serde::Serialize;

/// One line for readers of the deliverables; the metrics caveats and the review report have the
/// long form.
pub const LEAKAGE_CAVEAT: &str =
    "Held-out metrics are optimistic: every signal calibrated its scores on the same Questie ground truth the metrics use (see relations-review.md).";

/// A candidate file combine read, as relations.json and the meta list it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputSummary {
    pub signal: String,
    pub generated_at: String,
    pub input_count: usize,
    pub candidates: usize,
    pub covered: usize,
}

impl InputSummary {
    pub fn of(file: &CandidateFile) -> InputSummary {
        InputSummary {
            signal: file.signal.clone(),
            generated_at: file.generated_at.clone(),
            input_count: file.input_count,
            candidates: file.candidates.len(),
            covered: file.covered_quest_ids.len(),
        }
    }
}

/// What foreverQuestRelationTraces.lua changes: quests with a code line, and applied relation values
/// (QuestieDB targets kept in a merged list included). A breadcrumb or exclusive pair counts once
/// per quest it is written to. Commented-out relations are not counted.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct LuaCounts {
    pub quests: usize,
    pub relations: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct StatusCounts {
    pub agree: usize,
    pub new: usize,
    pub conflict: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationsMeta {
    pub generated_at: String,
    /// Acceptance threshold: relations at or above it are in the Lua module.
    pub min_score: f64,
    /// Episodes in episodes.jsonl that the trace signals read (Forever and unversioned builds),
    /// and the characters behind them.
    pub episode_count: usize,
    pub character_count: usize,
    pub lua: LuaCounts,
    /// Accepted relations on Forever-new quests with authored relations, by status against the
    /// authored data.
    pub authored: StatusCounts,
    /// Candidate files the model used.
    pub inputs: Vec<InputSummary>,
    /// Shown to reviewers as context only (Wowhead), never scored.
    pub context_only: Vec<String>,
    pub caveat: &'static str,
}

/// Counts the same episodes the signals read ([`forever_episodes`]), and their distinct
/// characters: (episodes, characters).
pub fn count_episodes(episodes: &[QuestEpisode]) -> (usize, usize) {
    let read = forever_episodes(episodes);
    let characters: FxHashSet<&str> = read.iter().map(|episode| episode.character_key.as_str()).collect();
    (read.len(), characters.len())
}

/// Relation values the Lua module applies: each id of an applied list field, one per applied
/// scalar field. Cleared and commented-out fields are not counted.
pub fn lua_relation_count(records: &LuaRecords) -> usize {
    records
        .values()
        .flatten()
        .filter(|written| written.written == Written::Applied)
        .map(|written| if written.field.is_scalar() { 1 } else { written.values.len() })
        .sum()
}

/// Quests the Lua module writes at least one code line for (applied or cleared).
pub fn lua_quest_count(records: &LuaRecords) -> usize {
    records.values().filter(|fields| fields.iter().any(|written| written.written.is_code())).count()
}

pub fn authored_status_counts<'v>(views: impl IntoIterator<Item = &'v QuestView<'v>>) -> StatusCounts {
    let mut counts = StatusCounts::default();
    for view in views
        .into_iter()
        .filter(|view| view.domain == Domain::Forever && view.reference == Reference::Authored)
    {
        for status in view.edges.iter().filter(|edge| edge.is_written()).filter_map(|edge| edge.status) {
            match status {
                Status::Agree => counts.agree += 1,
                Status::New => counts.new += 1,
                Status::Conflict => counts.conflict += 1,
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::edges::{EdgeKey, EdgeKind};
    use crate::combine::resolve::Outcome;
    use crate::combine::views::EdgeView;
    use crate::model::{QuestId, RelationField, RelationSet};
    use crate::test_support::make_episode;

    #[test]
    fn counts_the_episodes_the_signals_read_and_each_character_once() {
        let episode = |character: &str, interface_version: Option<i64>| QuestEpisode {
            character_key: character.into(),
            interface_version,
            ..make_episode()
        };
        let episodes = [
            episode("a", Some(16001)),
            episode("a", Some(16002)),
            episode("b", None),
            // Old Classic trace: the signals skip it, so the summary must too.
            episode("c", Some(11507)),
        ];

        assert_eq!(count_episodes(&episodes), (3, 2));
    }

    #[test]
    fn counts_every_id_of_an_applied_list_field_and_one_per_applied_scalar_field() {
        use super::super::lua::{LuaField, NotApplied};
        let field = |field: RelationField, values: Vec<QuestId>, written: Written| LuaField { field, values, written };
        let records = LuaRecords::from([
            (
                92000,
                vec![
                    field(RelationField::PreQuestSingle, vec![], Written::Cleared),
                    field(RelationField::PreQuestGroup, vec![1, 2], Written::Applied),
                    field(RelationField::ExclusiveTo, vec![92001, 92002], Written::Shown(NotApplied::AlreadyInQuestieDb)),
                ],
            ),
            (92001, vec![field(RelationField::NextQuestInChain, vec![92003], Written::Applied)]),
            (
                92002,
                vec![field(RelationField::NextQuestInChain, vec![92004], Written::Shown(NotApplied::AuthoredFix))],
            ),
        ]);

        // preQuestGroup's two ids and the chain; the cleared and the commented-out fields change nothing.
        assert_eq!(lua_relation_count(&records), 3);
        // 92002 is commented out as a whole.
        assert_eq!(lua_quest_count(&records), 2);
    }

    #[test]
    fn counts_written_relations_on_authored_forever_new_quests_only() {
        let edge = |outcome: Outcome, status: Option<Status>| EdgeView {
            key: EdgeKey::new(EdgeKind::Prerequisite, 0, 1),
            field: RelationField::PreQuestSingle,
            target: 1,
            probability: 0.95,
            held_out: true,
            outcome,
            because: None,
            status,
            claims: &[],
            context: &[],
            contributions: &[],
        };
        let relations = RelationSet::new();
        let view = |quest_id: QuestId, domain: Domain, reference: Reference, edges: Vec<EdgeView<'static>>| QuestView {
            quest_id,
            name: None,
            domain,
            reference,
            relations: &relations,
            edges,
            reference_only: vec![],
            hints: vec![],
            note: None,
        };
        let views = [
            view(
                92000,
                Domain::Forever,
                Reference::Authored,
                vec![
                    edge(Outcome::Accepted, Some(Status::Agree)),
                    edge(Outcome::Derived, Some(Status::New)),
                    edge(Outcome::Clique, Some(Status::Conflict)),
                    edge(Outcome::BelowThreshold, None),
                ],
            ),
            view(92001, Domain::Forever, Reference::None, vec![edge(Outcome::Accepted, Some(Status::New))]),
            view(
                500,
                Domain::Classic,
                Reference::Inherited,
                vec![edge(Outcome::Accepted, Some(Status::Conflict))],
            ),
        ];

        assert_eq!(authored_status_counts(&views), StatusCounts { agree: 1, new: 1, conflict: 1 });
    }
}
