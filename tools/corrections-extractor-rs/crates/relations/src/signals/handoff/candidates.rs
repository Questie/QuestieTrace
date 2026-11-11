//! Turns per-pair character counts into scored candidates.
//!
//! - nextQuestInChain(P) = X comes only from pops. On Forever a pop is either the server offering
//!   P's NextQuestInChain or the giver auto-opening its only available quest. The second kind
//!   shows up as X having been offered while P was not completed (a "contradicted" pair), which is
//!   why contradicted pops score low and are dropped when P has another pop target.
//! - preQuestSingle(X) ∋ P comes from pops and from newly listed quests. A single character offered
//!   X while P was not completed (and no same-name variant or race/class rule explains it) proves P
//!   is not required, so contradicted pairs are not emitted at all.
//! - preQuestGroup instead, when X's observed prerequisites behave as ALL-of (see [`all_of_members`]).
//!
//! Scores are the measured precision of each evidence class against both ground-truth tiers
//! (inheritedClassic and authoredForever, pooled), so a score approximates the probability that
//! the edge is right. Most remaining false positives are breadcrumbs (P leads to X but is
//! optional) and Forever-new quests wired into Classic chains, which the Classic tier cannot know.

use super::tally::{Characters, HandoffTally, Pair, PairStats};
use super::variants::{is_parallel_copy, CompletionMoments, VariantEdge};
use crate::model::{Evidence, QuestId, RelationCandidate, RelationField};
use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrerequisiteScores {
    /// Clean pops or lists from 3+ characters: 0.97 Classic (n 599) / 0.99 Forever (n 160).
    pub three_characters: f64,
    /// Clean lists from 1-2 characters, no pop: 0.93 (n 15) / 0.96 (n 27).
    pub few_lists: f64,
    /// A clean pop from 1-2 characters: 0.82 (n 68) / 1.0 (n 1).
    pub few_pops: f64,
    /// Only observations with confounders: 0.67 (n 3) / 0.91 (n 11).
    pub confounded_only: f64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextInChainScores {
    /// Uncontradicted, no rival pop target, 3+ characters: 0.97 (n 494) / 0.90 (n 60).
    pub three_pops: f64,
    /// Uncontradicted, no rival pop target, 1-2 characters: 0.90 (n 73) / 0.50 (n 4).
    pub few_pops: f64,
    /// Uncontradicted, but only pops with confounders: too rare to measure.
    pub confounded_only: f64,
    /// Under 20% of the characters handed X got a pop rather than a list: 0.50 (n 2) / 0.50 (n 4).
    pub mostly_listed: f64,
    /// Uncontradicted with an uncontradicted rival pop target; scaled by its share of pops: 2/4.
    pub rivalled: f64,
    /// Contradicted as a prerequisite, P has no other pop target, 3+ characters: 0.68 (n 72) / 0.50 (n 10).
    pub contradicted_three_pops: f64,
    /// Same with 1-2 characters: 0.24 (n 46) / 0.0 (n 2). Contradicted with other pop targets: 0/59, not emitted.
    pub contradicted_few_pops: f64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scores {
    pub pre_quest_single: PrerequisiteScores,
    pub next_quest_in_chain: NextInChainScores,
}

/// Measured precision per evidence class (2026-10 data, see the handoff report). The first
/// figure is the Classic tier, the second the authored Forever tier.
pub const SCORES: Scores = Scores {
    pre_quest_single: PrerequisiteScores {
        three_characters: 0.97,
        few_lists: 0.95,
        few_pops: 0.82,
        confounded_only: 0.8,
    },
    next_quest_in_chain: NextInChainScores {
        three_pops: 0.96,
        few_pops: 0.88,
        confounded_only: 0.5,
        mostly_listed: 0.5,
        rivalled: 0.5,
        contradicted_three_pops: 0.66,
        contradicted_few_pops: 0.23,
    },
};

/// Candidates scoring below this are left out: they would only add noise to the combination.
const MIN_SCORE: f64 = 0.2;

/// Observed pairs whose quest needs ALL of its observed prerequisites. With ALL-of, whichever
/// prerequisite a character finishes last triggers the hand-off, so every hand-off from one of
/// them happens with the others already completed. With ANY-of, some character gets X right after
/// one of them while another is not done. Groups need 2+ uncontradicted prerequisites that pass
/// this test both ways for every pair; anything less stays preQuestSingle. Same-name copies are
/// never tested against each other (they are flagged completed together), so a group with copies
/// fails.
pub fn all_of_members(tally: &HandoffTally) -> FxHashSet<Pair> {
    let mut by_quest: FxHashMap<QuestId, Vec<&PairStats>> = FxHashMap::default();
    for stats in tally.pairs().filter(|stats| stats.support() > 0 && !stats.is_contradicted()) {
        by_quest.entry(stats.pair.quest).or_default().push(stats);
    }
    let mut members = FxHashSet::default();
    for group in by_quest.values().filter(|group| group.len() >= 2) {
        let all_of = group.iter().all(|stats| {
            group
                .iter()
                .all(|other| other.pair == stats.pair || needed_alongside(stats, other.pair.prerequisite))
        });
        if all_of {
            members.extend(group.iter().map(|stats| stats.pair));
        }
    }
    members
}

/// Every character handed X after turning in P had already completed `other`.
fn needed_alongside(stats: &PairStats, other: QuestId) -> bool {
    stats
        .companions
        .get(&other)
        .is_some_and(|states| !states.done.is_empty() && states.not_done.is_empty())
}

pub fn prerequisite_candidates(tally: &HandoffTally, all_of: &FxHashSet<Pair>) -> Vec<RelationCandidate> {
    tally
        .pairs()
        .filter(|stats| stats.support() > 0 && !stats.is_contradicted())
        .map(|stats| RelationCandidate {
            quest_id: stats.pair.quest,
            field: if all_of.contains(&stats.pair) {
                RelationField::PreQuestGroup
            } else {
                RelationField::PreQuestSingle
            },
            target: stats.pair.prerequisite,
            score: prerequisite_score(stats),
            support: stats.support() as u32,
            contradict: 0,
            evidence: stats.pick_evidence(),
            note: Some(stats.counts_note()),
        })
        .collect()
}

/// preQuestSingle edges through same-name copies of an observed, uncontradicted edge. Edges that
/// were observed directly, or contradicted by some character, are left to the observed rules, and
/// copies never seen completing together with the original are not used (see
/// [`CompletionMoments`]). ALL-of groups are not extended: a copy of one member would have to be
/// required too. A copy inherits the observed edge's score, which measured right: 0.96 (n 217) and
/// 1.0 (n 86) on the authored Forever tier. When several observed edges imply one copy edge, the
/// first with the highest score speaks for it.
pub fn variant_candidates(edges: &[VariantEdge], tally: &HandoffTally, moments: &CompletionMoments, all_of: &FxHashSet<Pair>) -> Vec<RelationCandidate> {
    let mut best: IndexMap<Pair, RelationCandidate, FxBuildHasher> = IndexMap::default();
    for edge in edges.iter().filter(|edge| is_parallel_copy(edge, moments)) {
        if tally.get(edge.pair).is_some_and(|own| own.support() > 0 || own.is_contradicted()) {
            continue;
        }
        let Some(observed) = tally.get(edge.observed) else { continue };
        if observed.support() == 0 || observed.is_contradicted() || all_of.contains(&edge.observed) {
            continue;
        }
        let score = prerequisite_score(observed);
        if best.get(&edge.pair).is_some_and(|candidate| candidate.score >= score) {
            continue;
        }
        let mut evidence = observed.pick_evidence();
        evidence.truncate(3);
        let observed_edge = format!("{} <- {}", observed.pair.quest, observed.pair.prerequisite);
        let kind = edge.kind.name();
        let copy = Evidence {
            reference: evidence.first().map(|first| first.reference.clone()).unwrap_or_default(),
            t: None,
            text: format!("same-name copy ({kind}) of observed {observed_edge}"),
        };
        evidence.insert(0, copy);
        best.insert(
            edge.pair,
            RelationCandidate {
                quest_id: edge.pair.quest,
                field: RelationField::PreQuestSingle,
                target: edge.pair.prerequisite,
                score,
                // Nobody was observed taking this copy; the support belongs to the observed edge.
                support: 0,
                contradict: 0,
                evidence,
                note: Some(format!("variant:{kind} of {observed_edge}; {}", observed.counts_note())),
            },
        );
    }
    best.into_values().collect()
}

fn prerequisite_score(stats: &PairStats) -> f64 {
    let scores = SCORES.pre_quest_single;
    let clean = stats.pop.union(&stats.list).count();
    if clean >= 3 {
        scores.three_characters
    } else if !stats.pop.is_empty() {
        scores.few_pops
    } else if !stats.list.is_empty() {
        scores.few_lists
    } else {
        scores.confounded_only
    }
}

pub fn next_in_chain_candidates(tally: &HandoffTally) -> Vec<RelationCandidate> {
    let mut by_prerequisite: IndexMap<QuestId, Vec<&PairStats>, FxBuildHasher> = IndexMap::default();
    for stats in tally.pairs().filter(|stats| !stats.pop_characters().is_empty()) {
        by_prerequisite.entry(stats.pair.prerequisite).or_default().push(stats);
    }

    let mut candidates = Vec::new();
    for group in by_prerequisite.values() {
        for stats in group {
            let rivals: Vec<&PairStats> = group.iter().copied().filter(|other| other.pair != stats.pair).collect();
            let score = next_in_chain_score(stats, &rivals);
            if score < MIN_SCORE {
                continue;
            }
            // Characters who saw a different quest pop after the same turn-in.
            let rival_characters: Characters = rivals
                .iter()
                .flat_map(|other| other.pop_characters())
                .filter(|character| !stats.pop.contains(character))
                .collect();
            candidates.push(RelationCandidate {
                quest_id: stats.pair.prerequisite,
                field: RelationField::NextQuestInChain,
                target: stats.pair.quest,
                score: round_to_thousandths(score),
                support: stats.pop_characters().len() as u32,
                contradict: rival_characters.len() as u32,
                evidence: stats.pick_evidence(),
                note: Some(stats.counts_note()),
            });
        }
    }
    candidates
}

fn next_in_chain_score(stats: &PairStats, rivals: &[&PairStats]) -> f64 {
    let scores = SCORES.next_quest_in_chain;
    if stats.is_contradicted() {
        return match stats.pop.len() {
            clean if clean == 0 || !rivals.is_empty() => 0.0,
            clean if clean >= 3 => scores.contradicted_three_pops,
            _ => scores.contradicted_few_pops,
        };
    }
    let pops = stats.pop_characters().len();
    // A contradicted rival is the giver auto-opening its only quest, not a competing chain claim.
    let clean_rivals: Vec<&&PairStats> = rivals.iter().filter(|other| !other.is_contradicted()).collect();
    if !clean_rivals.is_empty() {
        let rival_pops: usize = clean_rivals.iter().map(|other| other.pop_characters().len()).sum();
        return scores.rivalled * (pops as f64 / (pops + rival_pops) as f64);
    }
    // The server pops P's NextQuestInChain whenever it can, so a quest usually re-listed instead is not it.
    let listed = stats.list.union(&stats.list_confounded).count();
    if (pops as f64) < 0.2 * (pops + listed) as f64 {
        return scores.mostly_listed;
    }
    match stats.pop.len() {
        clean if clean >= 3 => scores.three_pops,
        clean if clean > 0 => scores.few_pops,
        _ => scores.confounded_only,
    }
}

/// Rounded to three decimals; halves round up (scores are never negative).
fn round_to_thousandths(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::QuestEpisode;
    use crate::signals::handoff::counter::{CounterKind, CounterObservation};
    use crate::signals::handoff::detect::{Confounder, HandoffKind, HandoffObservation};
    use crate::test_support::{make_episode, npc};

    fn handoff(kind: HandoffKind, turned_in: QuestId, offered: QuestId, confounders: Vec<Confounder>) -> HandoffObservation {
        HandoffObservation {
            kind,
            turned_in,
            offered,
            giver: npc(1),
            turn_in_at: 10.0,
            offered_at: 10.5,
            confounders,
        }
    }

    const POP: HandoffKind = HandoffKind::Pop;
    const LIST: HandoffKind = HandoffKind::List { previous_list_at: 5.0 };

    fn not_started(prerequisite: QuestId, quest: QuestId) -> CounterObservation {
        CounterObservation {
            kind: CounterKind::NotStarted,
            pair: Pair { prerequisite, quest },
            t: 1.0,
            giver: None,
        }
    }

    /// Every candidate as (field, quest, target), sorted like the candidates file.
    fn fields(tally: &HandoffTally) -> Vec<(RelationField, QuestId, QuestId)> {
        let mut candidates = prerequisite_candidates(tally, &all_of_members(tally));
        candidates.extend(next_in_chain_candidates(tally));
        candidates.iter().map(|c| (c.field, c.quest_id, c.target)).collect()
    }

    #[test]
    fn next_quest_in_chain_comes_only_from_pops_and_pre_quest_single_from_pops_and_lists() {
        let episodes = [make_episode(), make_episode()];
        let mut tally = HandoffTally::default();
        tally.add_handoff(&episodes[0], &handoff(POP, 1, 2, vec![]));
        tally.add_handoff(&episodes[1], &handoff(LIST, 1, 3, vec![]));
        assert_eq!(
            fields(&tally),
            [
                (RelationField::PreQuestSingle, 2, 1),
                (RelationField::PreQuestSingle, 3, 1),
                (RelationField::NextQuestInChain, 1, 2)
            ]
        );
    }

    #[test]
    fn support_counts_a_character_once_however_many_of_its_sessions_show_the_hand_off() {
        let alice = QuestEpisode {
            character_key: "alice".into(),
            ..make_episode()
        };
        let alice_again = QuestEpisode {
            character_key: "alice".into(),
            ..make_episode()
        };
        let bob = make_episode();
        let mut tally = HandoffTally::default();
        for episode in [&alice, &alice_again, &bob] {
            tally.add_handoff(episode, &handoff(POP, 1, 2, vec![]));
        }
        let candidates = [prerequisite_candidates(&tally, &FxHashSet::default()), next_in_chain_candidates(&tally)].concat();
        assert_eq!(candidates.iter().map(|c| c.support).collect::<Vec<_>>(), [2, 2]);
        assert_eq!(candidates[0].evidence.len(), 2, "one example per counted character");
    }

    #[test]
    fn clean_evidence_scores_above_confounded_and_contradicted_prerequisites_are_dropped() {
        let episodes: Vec<QuestEpisode> = (0..4).map(|_| make_episode()).collect();
        let mut tally = HandoffTally::default();
        tally.add_handoff(&episodes[0], &handoff(LIST, 1, 2, vec![]));
        tally.add_handoff(&episodes[1], &handoff(LIST, 1, 3, vec![Confounder::LevelUp { from: 10, to: 11 }]));
        tally.add_handoff(&episodes[2], &handoff(LIST, 1, 4, vec![]));
        tally.add_counter(&episodes[3], &not_started(1, 4));
        let scores: Vec<(QuestId, f64)> = prerequisite_candidates(&tally, &FxHashSet::default())
            .iter()
            .map(|c| (c.quest_id, c.score))
            .collect();
        assert_eq!(scores, [(2, SCORES.pre_quest_single.few_lists), (3, SCORES.pre_quest_single.confounded_only)]);
    }

    #[test]
    fn pre_quest_group_when_every_hand_off_from_one_prerequisite_found_the_others_done() {
        // X = 9 after 1 and 2. All-of: whoever finished 1 last had 2 done, and the other way round.
        // Any-of: one character got 9 right after 1 with 2 not done.
        let fields_with = |second_done_after_first: bool| {
            let (first, second) = (make_episode(), make_episode());
            let mut tally = HandoffTally::default();
            tally.add_handoff(&first, &handoff(POP, 1, 9, vec![]));
            tally.add_handoff(&second, &handoff(POP, 2, 9, vec![]));
            tally.add_companion_states(&first, Pair { prerequisite: 1, quest: 9 }, [(2, second_done_after_first)]);
            tally.add_companion_states(&second, Pair { prerequisite: 2, quest: 9 }, [(1, true)]);
            prerequisite_candidates(&tally, &all_of_members(&tally))
                .into_iter()
                .map(|c| c.field)
                .collect::<Vec<_>>()
        };
        assert_eq!(fields_with(true), [RelationField::PreQuestGroup, RelationField::PreQuestGroup]);
        assert_eq!(fields_with(false), [RelationField::PreQuestSingle, RelationField::PreQuestSingle]);
    }

    #[test]
    fn a_contradicted_pop_is_no_next_quest_in_chain_when_the_same_turn_in_popped_something_else() {
        // P = 1 popped 2 for three characters; 3 popped once but was offered elsewhere without 1:
        // the giver auto-opening its only quest, not a chain.
        let episodes: Vec<QuestEpisode> = (0..5).map(|_| make_episode()).collect();
        let mut tally = HandoffTally::default();
        for episode in &episodes[..3] {
            tally.add_handoff(episode, &handoff(POP, 1, 2, vec![]));
        }
        tally.add_handoff(&episodes[3], &handoff(POP, 1, 3, vec![]));
        tally.add_counter(&episodes[4], &not_started(1, 3));
        let chain: Vec<_> = next_in_chain_candidates(&tally).iter().map(|c| (c.target, c.score, c.contradict)).collect();
        assert_eq!(
            chain,
            [(2, SCORES.next_quest_in_chain.three_pops, 1)],
            "a contradicted rival does not split the score"
        );
    }

    #[test]
    fn rival_pops_after_the_same_turn_in_split_next_quest_in_chain_confidence() {
        let episodes: Vec<QuestEpisode> = (0..5).map(|_| make_episode()).collect();
        let mut tally = HandoffTally::default();
        for episode in &episodes[..3] {
            tally.add_handoff(episode, &handoff(POP, 1, 2, vec![]));
        }
        for episode in &episodes[3..] {
            tally.add_handoff(episode, &handoff(POP, 1, 3, vec![]));
        }
        let chain: Vec<_> = next_in_chain_candidates(&tally)
            .iter()
            .map(|c| (c.target, c.score, c.support, c.contradict))
            .collect();
        // rivalled (0.5) times the share of pops: 3/5 and 2/5.
        assert_eq!(chain, [(2, 0.3, 3, 2), (3, 0.2, 2, 3)]);
    }

    #[test]
    fn a_quest_usually_listed_rather_than_popped_is_no_likely_next_quest_in_chain() {
        let episodes: Vec<QuestEpisode> = (0..6).map(|_| make_episode()).collect();
        let mut tally = HandoffTally::default();
        tally.add_handoff(&episodes[0], &handoff(POP, 1, 2, vec![]));
        for episode in &episodes[1..] {
            tally.add_handoff(episode, &handoff(LIST, 1, 2, vec![]));
        }
        let chain: Vec<_> = next_in_chain_candidates(&tally).iter().map(|c| c.score).collect();
        assert_eq!(chain, [SCORES.next_quest_in_chain.mostly_listed]);
    }

    #[test]
    fn pops_with_confounders_still_claim_next_quest_in_chain_at_a_lower_score() {
        let episode = make_episode();
        let mut tally = HandoffTally::default();
        tally.add_handoff(&episode, &handoff(POP, 1, 2, vec![Confounder::AlsoTurnedIn(3)]));
        let chain: Vec<_> = next_in_chain_candidates(&tally).iter().map(|c| (c.target, c.score, c.support)).collect();
        assert_eq!(chain, [(2, SCORES.next_quest_in_chain.confounded_only, 1)]);
    }
}
