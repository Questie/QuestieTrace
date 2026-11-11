//! `relations signal:cooccurrence`: completion histories across characters -> prerequisite
//! (implication) and exclusiveTo (exclusion) candidates.
//!
//! Every session records the character's full completed set at capture start plus every change,
//! so this signal reaches quests no giver interaction was recorded for. Pipeline: [`history`]
//! (sessions -> one ordered history per character), [`population`] (dense indexes),
//! [`prerequisites`], [`coflag`] and [`exclusion`] (inference), [`calibration`] (features ->
//! score). Writes `candidates/cooccurrence.json` and `reports/cooccurrence-summary.json`.

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
use self::history::{build_histories, unstable_quests, HistoryParams};
use self::population::Population;
use self::prerequisites::{infer_prerequisites, PrerequisiteEdge, PrerequisiteParams};
use super::{forever_episodes, to_json, Findings, Signal, SignalInput};
use crate::eligibility::is_forever_quest_id;
use crate::model::{Evidence, QuestCatalog, QuestEpisode, QuestId, RelationCandidate, RelationField};
use crate::text::{to_exponential, to_fixed};
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

    fn run(&self, input: &SignalInput) -> Findings {
        let episodes = forever_episodes(input.episodes);
        let Analysis {
            candidates,
            covered,
            mut report,
        } = analyze(&episodes, input.catalog, &PARAMS);
        report["episodesRead"] = json!(input.episodes.len());
        let params = FileParams {
            tunables: &PARAMS,
            support: "prerequisites: takers of the quest who could take the target; exclusiveTo: completers of either quest who could take the other",
            contradict: "always 0 for prerequisites and never-together exclusions (one contradiction drops the edge); co-flags: 0 as well",
        };
        Findings {
            input_count: episodes.len(),
            params: to_json(&params),
            covered_quest_ids: covered,
            candidates,
            reports: vec![("cooccurrence-summary", report)],
        }
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
}

/// The whole signal on the (Forever) episodes given.
pub fn analyze(episodes: &[&QuestEpisode], catalog: &QuestCatalog, params: &Params) -> Analysis {
    let ignore = unstable_quests(episodes, catalog);
    let (histories, history_report) = build_histories(episodes, &params.history, &ignore);
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
    Analysis { candidates, covered, report }
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
    use crate::signals::cooccurrence::fixtures::{catalog_of, session, SessionSpec};

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
        let episodes: Vec<&QuestEpisode> = episodes.iter().collect();
        let analysis = analyze(&episodes, &catalog_of([8, 41, 42]), &PARAMS);

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
