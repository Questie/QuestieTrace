//! `relations signal:cooccurrence`: completion histories across characters -> prerequisite
//! (implication) and exclusiveTo (exclusion) candidates.
//!
//! Every session records the character's full completed set at capture start plus every change,
//! so this signal reaches quests no giver interaction was recorded for. Pipeline: [`history`]
//! (sessions -> one ordered history per character, built one bucket of episodes at a time),
//! [`population`] (dense indexes), [`prerequisites`], [`coflag`] and [`exclusion`] (inference),
//! [`calibration`] (features -> score). Writes `candidates/cooccurrence.json` and
//! `reports/cooccurrence-summary.json`.

mod calibration;
mod coflag;
mod exclusion;
#[cfg(test)]
mod fixtures;
mod history;
mod population;
mod prerequisites;

use self::calibration::{co_flag_score, exclusion_score, prerequisite_score};
use self::coflag::{infer_co_flags, CoFlagEdge, CoFlagParams, CoFlagged};
use self::exclusion::{infer_exclusions, ExclusionEdge, ExclusionKind, ExclusionParams};
use self::history::{add_unstable_quests, build_histories, repeatable_quests, CharacterHistory, HistoryParams, HistoryReport};
use self::population::Population;
use self::prerequisites::{infer_prerequisites, PrerequisiteEdge, PrerequisiteParams};
use super::{forever_episodes, to_json, Findings, Signal, SignalInput};
use crate::eligibility::is_forever_quest_id;
use crate::episode_store::Episodes;
use crate::model::{Evidence, QuestCatalog, QuestId, RelationCandidate, RelationField};
use crate::text::{to_exponential, to_fixed};
use anyhow::Result;
use rustc_hash::FxHashSet;
use serde::Serialize;
use serde_json::json;

#[derive(Clone, Debug, Serialize)]
pub struct Params {
    pub history: HistoryParams,
    pub prerequisites: PrerequisiteParams,
    pub coflag: CoFlagParams,
    pub exclusion: ExclusionParams,
}

pub const PARAMS: Params = Params {
    history: HistoryParams { max_initial_loss: 0.1 },
    prerequisites: PrerequisiteParams {
        min_takers: 3,
        min_ordered: 1,
        max_null_probability: 0.05,
        order_tolerance_seconds: 10.0,
        handoff_seconds: 300.0,
    },
    coflag: CoFlagParams {
        window_seconds: 3.0,
        min_observations: 2,
    },
    exclusion: ExclusionParams {
        min_completers: 3,
        min_expected: 6.0,
        min_group_expected: 3.0,
        min_group_member_completers: 2,
    },
};

/// candidates/cooccurrence.json `params`: the tunables and what the counts mean.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileParams<'a> {
    #[serde(flatten)]
    tunables: &'a Params,
    support: &'static str,
    contradict: &'static str,
}

pub struct Cooccurrence;

impl Signal for Cooccurrence {
    fn name(&self) -> &'static str {
        "cooccurrence"
    }

    fn describe(&self) -> &'static str {
        "completion histories across characters -> implication/exclusion"
    }

    fn run(&self, input: &SignalInput) -> Result<Findings> {
        let Analysis {
            candidates,
            covered,
            mut report,
            input_count,
            episodes_read,
        } = analyze(input.episodes, input.catalog, &PARAMS)?;
        report["episodesRead"] = json!(episodes_read);
        let params = FileParams {
            tunables: &PARAMS,
            support: "prerequisites: takers of the quest who could take the target; exclusiveTo: completers of either quest who could take the other",
            contradict: "always 0 for prerequisites and never-together exclusions (one contradiction drops the edge); co-flags: 0 as well",
        };
        Ok(Findings {
            input_count,
            params: to_json(&params),
            covered_quest_ids: covered,
            candidates,
            reports: vec![("cooccurrence-summary", report)],
        })
    }
}

/// The signal's result without I/O.
pub struct Analysis {
    /// In [`RelationCandidate::canonical_order`].
    pub candidates: Vec<RelationCandidate>,
    /// Only quests judged for both fields, so neither field's recall counts quests it never saw.
    pub covered: Vec<QuestId>,
    /// reports/cooccurrence-summary.json, less `episodesRead`.
    pub report: serde_json::Value,
    /// The Forever episodes the histories were built from.
    pub input_count: usize,
    /// Every episode read, Forever or not.
    pub episodes_read: usize,
}

/// The whole signal on the episodes given (their Forever ones), read one bucket at a time.
pub fn analyze(episodes: &dyn Episodes, catalog: &QuestCatalog, params: &Params) -> Result<Analysis> {
    let Histories {
        histories,
        report: history_report,
        ignore,
        episodes_read,
    } = read_histories(episodes, catalog, &params.history)?;
    let input_count = history_report.episodes;
    let population = Population::new(histories, catalog, &ignore);
    let coflags = infer_co_flags(&population, &params.coflag);
    let co_flagged = CoFlagged::new(coflags.edges.iter().map(|edge| (edge.a, edge.b)));
    let prerequisites = infer_prerequisites(&population, &params.prerequisites, &co_flagged);
    let exclusions = infer_exclusions(&population, &params.exclusion);

    let mut candidates: Vec<RelationCandidate> = prerequisites.edges.iter().map(prerequisite_candidate).collect();
    candidates.extend(coflags.edges.iter().flat_map(co_flag_candidates));
    candidates.extend(exclusions.edges.iter().flat_map(exclusion_candidates));
    candidates.sort_by_key(RelationCandidate::canonical_order);

    let co_flag_covered: FxHashSet<QuestId> = coflags.covered.iter().copied().collect();
    let covered: Vec<QuestId> = prerequisites
        .covered
        .iter()
        .copied()
        .filter(|quest_id| co_flag_covered.contains(quest_id))
        .collect();
    let either_forever = |a: QuestId, b: QuestId| is_forever_quest_id(a) || is_forever_quest_id(b);
    let report = json!({
        "history": history_report,
        "ignoredUnstableQuests": ignore.len(),
        "coveredBoth": covered.len(),
        "prerequisites": {
            "covered": prerequisites.covered.len(),
            "edges": prerequisites.edges.len(),
            "pruned": prerequisites.pruned,
            "uncoveredVariants": prerequisites.uncovered_variants,
        },
        "coflag": {
            "covered": coflags.covered.len(),
            "observedPairs": coflags.observed_pairs,
            "contradicted": coflags.contradicted,
            "pairs": coflags.edges.len(),
        },
        "exclusion": {
            "testedPairs": exclusions.tested_pairs,
            "conflicted": exclusions.conflicted,
            "pairs": exclusions.edges.len(),
        },
        "foreverNew": {
            "prerequisiteEdges": prerequisites.edges.iter().filter(|edge| is_forever_quest_id(edge.implication.quest_id)).count(),
            "coflagPairs": coflags.edges.iter().filter(|edge| either_forever(edge.a, edge.b)).count(),
            "exclusionPairs": exclusions.edges.iter().filter(|edge| either_forever(edge.pair.a, edge.pair.b)).count(),
        },
    });
    Ok(Analysis {
        candidates,
        covered,
        report,
        input_count,
        episodes_read,
    })
}

/// What the passes over the episodes leave for the inference.
struct Histories {
    /// Every character's history, characters in the order they are read.
    histories: Vec<CharacterHistory>,
    report: HistoryReport,
    /// The unstable quests, which the histories leave out (see [`repeatable_quests`]).
    ignore: FxHashSet<QuestId>,
    episodes_read: usize,
}

/// Reads the episodes twice, one bucket at a time: the quests to ignore depend on every episode,
/// and the histories leave them out. A bucket holds whole characters, so each history is built
/// from one bucket and owns what it needs; the bucket goes once its histories are built.
fn read_histories(episodes: &dyn Episodes, catalog: &QuestCatalog, params: &HistoryParams) -> Result<Histories> {
    let mut ignore = repeatable_quests(catalog);
    let mut episodes_read = 0;
    episodes.for_each_bucket(&mut |bucket| {
        episodes_read += bucket.len();
        add_unstable_quests(&mut ignore, &forever_episodes(bucket));
        Ok(())
    })?;

    let mut histories = Vec::new();
    let mut report = HistoryReport::default();
    episodes.for_each_bucket(&mut |bucket| {
        let (built, built_report) = build_histories(&forever_episodes(bucket), params, &ignore);
        histories.extend(built);
        report += built_report;
        Ok(())
    })?;
    Ok(Histories {
        histories,
        report,
        ignore,
        episodes_read,
    })
}

fn prerequisite_candidate(edge: &PrerequisiteEdge) -> RelationCandidate {
    let implication = &edge.implication;
    RelationCandidate {
        quest_id: implication.quest_id,
        field: edge.field,
        target: implication.prereq,
        score: prerequisite_score(edge),
        support: count(implication.takers),
        contradict: 0,
        evidence: implication.witnesses.clone(),
        note: Some(format!(
            "takers={}/{} ordered={} unordered={} baseline={}/{} null={} shape={} siblings={} rank={} handoffs={}/{} levelUps={}",
            implication.takers,
            edge.all_takers,
            implication.ordered,
            implication.unordered,
            edge.baseline.completed,
            edge.baseline.of,
            to_exponential(edge.null_probability, 1),
            edge.shape.name(),
            edge.siblings,
            edge.rank,
            edge.handoffs,
            implication.gaps.len(),
            implication.level_ups
        )),
    }
}

fn exclusion_candidates(edge: &ExclusionEdge) -> [RelationCandidate; 2] {
    let pair = &edge.pair;
    let kind = match edge.kind {
        ExclusionKind::Pair => "pair".to_owned(),
        ExclusionKind::Group { size, expected } => format!("group groupSize={size} groupExpected={}", to_fixed(expected, 1)),
    };
    let note = format!(
        "completers={}+{} expected={} kind={kind} switched={}",
        pair.completers_a,
        pair.completers_b,
        to_fixed(pair.expected, 1),
        edge.switched
    );
    both_directions(
        pair.a,
        pair.b,
        Claim {
            score: exclusion_score(edge),
            support: count(pair.completers_a + pair.completers_b),
            contradict: 0,
            evidence: &edge.evidence,
            note,
        },
    )
}

fn co_flag_candidates(edge: &CoFlagEdge) -> [RelationCandidate; 2] {
    both_directions(
        edge.a,
        edge.b,
        Claim {
            score: co_flag_score(edge),
            support: edge.observations + edge.unordered,
            contradict: edge.contradict,
            evidence: &edge.evidence,
            note: format!(
                "kind=coflag observations={} turnedIn={}+{} unordered={}",
                edge.observations, edge.turned_in_a, edge.turned_in_b, edge.unordered
            ),
        },
    )
}

/// What an exclusiveTo candidate claims, on either quest of the pair.
struct Claim<'a> {
    score: f64,
    support: u32,
    contradict: u32,
    evidence: &'a [Evidence],
    note: String,
}

/// Questie's exclusiveTo is one-directional, so every exclusion is emitted on both quests.
fn both_directions(a: QuestId, b: QuestId, claim: Claim) -> [RelationCandidate; 2] {
    let candidate = |quest_id: QuestId, target: QuestId| RelationCandidate {
        quest_id,
        field: RelationField::ExclusiveTo,
        target,
        score: claim.score,
        support: claim.support,
        contradict: claim.contradict,
        evidence: claim.evidence.to_vec(),
        note: Some(claim.note.clone()),
    };
    [candidate(a, b), candidate(b, a)]
}

/// A character count as a candidate's support.
fn count(characters: usize) -> u32 {
    u32::try_from(characters).expect("fewer than 4 billion characters")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode_store::{InMemory, PerCharacter};
    use crate::model::{CompletedChange, QuestEpisode};
    use crate::signals::cooccurrence::fixtures::{catalog_of, session, SessionSpec};

    /// Characters `keys` each doing `spec`.
    fn characters(keys: &[&str], spec: impl Fn() -> SessionSpec) -> Vec<QuestEpisode> {
        keys.iter().map(|key| session(key, spec())).collect()
    }

    #[test]
    fn reads_the_same_bucket_by_bucket_as_all_at_once() {
        // a-c complete 9, then hand 2 off to 3; d-f turn in 41, which flags its copy 42; g-i did
        // nothing relevant; y only ever completed 9. z, read last, shows 9 resetting: 9 is ignored
        // from the first character on, so y has no history at all.
        let mut episodes = characters(&["a", "b", "c"], || SessionSpec {
            initial: vec![8],
            turn_ins: vec![(50.0, 9), (100.0, 2)],
            accepts: vec![(101.0, 3)],
            ..SessionSpec::default()
        });
        episodes.extend(characters(&["d", "e", "f"], || SessionSpec {
            initial: vec![8],
            turn_ins: vec![(100.0, 41)],
            flags: vec![(100.5, 42)],
            ..SessionSpec::default()
        }));
        episodes.extend(characters(&["g", "h", "i"], || SessionSpec {
            initial: vec![8],
            ..SessionSpec::default()
        }));
        episodes.extend(characters(&["y"], || SessionSpec {
            initial: vec![9],
            ..SessionSpec::default()
        }));
        let mut reset = session(
            "z",
            SessionSpec {
                initial: vec![8, 9],
                ..SessionSpec::default()
            },
        );
        if let Some(completed) = &mut reset.completed {
            completed.changes.push(CompletedChange {
                t: 10.0,
                add: None,
                remove: Some(vec![9]),
            });
        }
        episodes.push(reset);
        let catalog = catalog_of([2, 3, 8, 9, 41, 42]);

        let together = analyze(&InMemory(&episodes), &catalog, &PARAMS).unwrap();
        let claims: Vec<(QuestId, RelationField, QuestId)> = together
            .candidates
            .iter()
            .map(|candidate| (candidate.quest_id, candidate.field, candidate.target))
            .collect();
        assert_eq!(
            claims,
            [
                (3, RelationField::PreQuestSingle, 2),
                (41, RelationField::ExclusiveTo, 42),
                (42, RelationField::ExclusiveTo, 41)
            ],
            "9 is no prerequisite of 2: it is unstable"
        );
        assert_eq!(together.report["history"]["characters"], 10, "every character but y");
        assert_eq!(
            (together.input_count, together.episodes_read),
            (episodes.len(), episodes.len()),
            "every episode read once"
        );

        let apart = analyze(&PerCharacter(&episodes), &catalog, &PARAMS).unwrap();
        assert_eq!(apart.candidates, together.candidates);
        assert_eq!(apart.covered, together.covered);
        assert_eq!(apart.report, together.report);
        assert_eq!((apart.input_count, apart.episodes_read), (together.input_count, together.episodes_read));
    }

    #[test]
    fn co_flags_are_claimed_on_both_quests_and_cover_only_quests_judged_for_both_fields() {
        let episodes: Vec<QuestEpisode> = ["a", "b", "c"]
            .iter()
            .map(|key| {
                session(
                    key,
                    SessionSpec {
                        initial: vec![8],
                        turn_ins: vec![(100.0, 41)],
                        flags: vec![(100.5, 42)],
                        ..SessionSpec::default()
                    },
                )
            })
            .collect();
        let analysis = analyze(&InMemory(&episodes), &catalog_of([8, 41, 42]), &PARAMS).unwrap();

        let claims: Vec<(QuestId, RelationField, QuestId, u32)> = analysis
            .candidates
            .iter()
            .map(|candidate| (candidate.quest_id, candidate.field, candidate.target, candidate.support))
            .collect();
        assert_eq!(claims, [(41, RelationField::ExclusiveTo, 42, 3), (42, RelationField::ExclusiveTo, 41, 3)]);
        assert_eq!(
            analysis.candidates[0].note.as_deref(),
            Some("kind=coflag observations=3 turnedIn=3+0 unordered=0")
        );
        // 8 was only ever completed before capture, so co-flagging never judged it.
        assert_eq!(analysis.covered, [41, 42]);
    }
}
