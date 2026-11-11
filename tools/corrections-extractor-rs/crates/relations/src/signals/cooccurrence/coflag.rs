//! Exclusion from co-flagging: completing either quest of a server exclusive group marks its
//! siblings completed too, in the same instant.
//!
//! This is how most exclusiveTo groups actually show up in completion histories (measured on both
//! ground-truth tiers: e.g. every character with one "Camping 101: Mining" copy has all of them,
//! added in the same completed-set delta as the one it turned in). "Never together" is the
//! textbook signature but is rare in practice; see [`super::exclusion`].
//!
//! An observation is one character whose completed set gained both quests within `window_seconds`
//! while it turned in at most one of them. Moments are the completed set's ([`Stamp::flagged`]),
//! not the turn-in: siblings only ever show up there, and the sample can trail the turn-in by many
//! seconds. Separate exact turn-ins still contradict co-flagging, even if one late sample batches
//! them. A pair qualifies when enough characters show that, every character who completed either
//! has both, at the same moment as far as can be seen, and nobody ever had both active at once
//! (which exclusiveTo would have hidden). A breadcrumb flagged by its target's turn-in fails that
//! check, because characters who took the breadcrumb itself completed it earlier than the target.

use crate::text::to_fixed;
use super::history::{held_together, Stamp, When};
use super::population::{Character, Population};
use crate::model::{Evidence, QuestEventKind, QuestId, SessionTime};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoFlagParams {
    /// Completions this close together in one session count as one server-side flag.
    pub window_seconds: SessionTime,
    /// Characters observed gaining both at once while turning in at most one.
    pub min_observations: usize,
}

pub struct CoFlagEdge {
    /// The lower quest id of the pair.
    pub a: QuestId,
    pub b: QuestId,
    /// Characters seen gaining both at once while turning in at most one.
    pub observations: u32,
    /// Of those, characters that turned in `a` / `b` (the other was flagged). Both > 0 = symmetric.
    pub turned_in_a: u32,
    pub turned_in_b: u32,
    /// Characters with both already completed when a session started (consistent, but no timing).
    pub unordered: u32,
    /// Characters with only one of them, or both at clearly different times.
    pub contradict: u32,
    pub evidence: Vec<Evidence>,
}

pub struct CoFlagResult {
    pub edges: Vec<CoFlagEdge>,
    /// Quests completed during a recorded session by at least `min_observations` characters: had
    /// they a co-flagged sibling, it would have been seen landing with them.
    pub covered: Vec<QuestId>,
    /// Pairs with at least one observation.
    pub observed_pairs: usize,
    /// Observed pairs dropped because some character completed one without the other.
    pub contradicted: usize,
}

/// Pairs of copies the server completes together, in either order.
#[derive(Default)]
pub struct CoFlagged(FxHashSet<(QuestId, QuestId)>);

impl CoFlagged {
    pub fn new(pairs: impl IntoIterator<Item = (QuestId, QuestId)>) -> Self {
        CoFlagged(pairs.into_iter().map(|(a, b)| (a.min(b), a.max(b))).collect())
    }

    pub fn contains(&self, a: QuestId, b: QuestId) -> bool {
        self.0.contains(&(a.min(b), a.max(b)))
    }
}

/// All characters' observations of one pair.
struct Observed {
    characters: u32,
    turned_in_a: u32,
    turned_in_b: u32,
    /// The first three observing characters' evidence.
    evidence: Vec<Evidence>,
}

/// One character gaining a pair together. The pair is two quest indexes, the lower first (so also
/// the lower quest id first).
struct Observation {
    pair: (usize, usize),
    turned_in_a: bool,
    turned_in_b: bool,
    evidence: Evidence,
}

pub fn infer_co_flags(population: &Population, params: &CoFlagParams) -> CoFlagResult {
    let mut observed: FxHashMap<(usize, usize), Observed> = FxHashMap::default();
    for character in &population.characters {
        for observation in observations_of(character, population, params.window_seconds) {
            let total = observed.entry(observation.pair).or_insert_with(|| Observed {
                characters: 0,
                turned_in_a: 0,
                turned_in_b: 0,
                evidence: Vec::new(),
            });
            total.characters += 1;
            total.turned_in_a += u32::from(observation.turned_in_a);
            total.turned_in_b += u32::from(observation.turned_in_b);
            if total.evidence.len() < 3 {
                total.evidence.push(observation.evidence);
            }
        }
    }

    let covered = population
        .quests
        .iter()
        .zip(&population.completers)
        .filter(|(quest, completers)| {
            let seen_landing = completers
                .iter()
                .filter(|&&character| !population.characters[character].history.done[&quest.id].is_at_start())
                .count();
            seen_landing >= params.min_observations
        })
        .map(|(quest, _)| quest.id)
        .collect();

    let mut result = CoFlagResult {
        edges: Vec::new(),
        covered,
        observed_pairs: observed.len(),
        contradicted: 0,
    };
    let mut candidates: Vec<((usize, usize), Observed)> = observed
        .into_iter()
        .filter(|(_, observation)| observation.characters as usize >= params.min_observations)
        .collect();
    candidates.sort_unstable_by_key(|(pair, _)| *pair);
    // Checking a pair walks every taker's quest logs, so the pairs are checked side by side.
    let checks: Vec<PairCheck> = candidates
        .par_iter()
        .map(|&((index_a, index_b), _)| check_pair(population, index_a, index_b, params.window_seconds))
        .collect();
    for (((index_a, index_b), observation), check) in candidates.into_iter().zip(checks) {
        if check.contradict > 0 {
            result.contradicted += 1;
            continue;
        }
        result.edges.push(CoFlagEdge {
            a: population.quests[index_a].id,
            b: population.quests[index_b].id,
            observations: observation.characters,
            turned_in_a: observation.turned_in_a,
            turned_in_b: observation.turned_in_b,
            unordered: check.unordered,
            contradict: check.contradict,
            evidence: observation.evidence,
        });
    }
    result
}

/// Pairs this character gained together, at most one observation per pair.
fn observations_of(character: &Character, population: &Population, window_seconds: SessionTime) -> Vec<Observation> {
    let history = &character.history;
    // Per session, the completions seen landing in it (quest index, completed-set time).
    let mut by_session: BTreeMap<usize, Vec<(usize, SessionTime)>> = BTreeMap::new();
    for &index in &character.done {
        let stamp = &history.done[&population.quests[index].id];
        if let Some(t) = stamp.flagged_time() {
            by_session.entry(stamp.session).or_default().push((index, t));
        }
    }

    let mut result = Vec::new();
    for (session, mut completions) in by_session {
        completions.sort_by(|x, y| x.1.total_cmp(&y.1));
        let episode = history.sessions[session].episode;
        let handed_in: FxHashSet<QuestId> = episode
            .quest_events
            .iter()
            .filter(|event| event.kind == QuestEventKind::TurnedIn)
            .map(|event| event.quest_id)
            .collect();
        for (i, &(first, first_t)) in completions.iter().enumerate() {
            for &(second, second_t) in completions[i + 1..].iter().take_while(|(_, t)| t - first_t <= window_seconds) {
                let pair = (first.min(second), first.max(second));
                let (a, b) = (population.quests[pair.0].id, population.quests[pair.1].id);
                if handed_in.contains(&a) && handed_in.contains(&b) {
                    continue;
                }
                let outcome = match [a, b].into_iter().find(|id| handed_in.contains(id)) {
                    Some(turned_in) => format!(", only {turned_in} turned in"),
                    None => ", neither turned in".to_owned(),
                };
                result.push(Observation {
                    pair,
                    turned_in_a: handed_in.contains(&a),
                    turned_in_b: handed_in.contains(&b),
                    evidence: Evidence {
                        reference: episode.key.clone(),
                        t: Some(first_t),
                        text: format!(
                            "character {}: {a} and {b} became completed {}s apart{outcome}",
                            history.key,
                            to_fixed(second_t - first_t, 1)
                        ),
                    },
                });
            }
        }
    }
    result
}

struct PairCheck {
    unordered: u32,
    contradict: u32,
}

/// Every completer of either quest must have both, completed at the same moment where visible, and
/// no taker of both may have held them at once.
fn check_pair(population: &Population, index_a: usize, index_b: usize, window_seconds: SessionTime) -> PairCheck {
    let (a, b) = (population.quests[index_a].id, population.quests[index_b].id);
    let mut completers: Vec<usize> = population.completers[index_a].iter().chain(&population.completers[index_b]).copied().collect();
    completers.sort_unstable();
    completers.dedup();

    let mut check = PairCheck { unordered: 0, contradict: 0 };
    for character in completers {
        let history = &population.characters[character].history;
        match (history.done.get(&a), history.done.get(&b)) {
            (Some(done_a), Some(done_b)) if done_a.is_at_start() && done_b.is_at_start() && done_a.session == done_b.session => check.unordered += 1,
            (Some(done_a), Some(done_b)) if same_moment(done_a, done_b, window_seconds) => {}
            _ => check.contradict += 1,
        }
    }
    for &character in &population.takers[index_a] {
        let history = &population.characters[character].history;
        if history.taken.contains_key(&b) && held_together(history, a, b) {
            check.contradict += 1;
        }
    }
    check
}

fn same_moment(a: &Stamp, b: &Stamp, window_seconds: SessionTime) -> bool {
    match (a.when, b.when) {
        // A delayed sample must not erase evidence that the character turned in each quest separately.
        (When::During { t: t_a, exact: true }, When::During { t: t_b, exact: true }) if (t_a - t_b).abs() > window_seconds => false,
        _ => match (a.flagged_time(), b.flagged_time()) {
            (Some(t_a), Some(t_b)) => a.session == b.session && (t_a - t_b).abs() <= window_seconds,
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CompletedChange;
    use crate::signals::cooccurrence::fixtures::{catalog_of, population_of, session, SessionSpec};

    const PARAMS: CoFlagParams = CoFlagParams {
        window_seconds: 3.0,
        min_observations: 2,
    };

    /// Characters `keys` each turn in `turned_in` at 100 and get `flagged` half a second later.
    fn flagged_by(keys: &[&str], turned_in: QuestId, flagged: QuestId) -> Vec<crate::model::QuestEpisode> {
        keys.iter()
            .map(|key| {
                session(
                    key,
                    SessionSpec {
                        initial: vec![8],
                        turn_ins: vec![(100.0, turned_in)],
                        flags: vec![(100.5, flagged)],
                        ..SessionSpec::default()
                    },
                )
            })
            .collect()
    }

    fn pairs(result: &CoFlagResult) -> Vec<(QuestId, QuestId)> {
        result.edges.iter().map(|edge| (edge.a, edge.b)).collect()
    }

    #[test]
    fn copies_the_server_completes_together_pair_up_whichever_one_was_turned_in() {
        let mut episodes = flagged_by(&["a", "b"], 41, 42);
        episodes.extend(flagged_by(&["c", "d"], 42, 41));
        let catalog = catalog_of([8, 41, 42]);
        let result = infer_co_flags(&population_of(&episodes, &catalog), &PARAMS);

        assert_eq!(pairs(&result), [(41, 42)]);
        let edge = &result.edges[0];
        assert_eq!((edge.observations, edge.turned_in_a, edge.turned_in_b, edge.contradict), (4, 2, 2, 0));
        assert_eq!(edge.evidence.len(), 3, "evidence from the first three observers");
        // Both landed in the completed set at 100.5: the turned-in 41 by its delayed delta.
        assert_eq!(edge.evidence[0].text, "character a: 41 and 42 became completed 0.0s apart, only 41 turned in");
    }

    #[test]
    fn a_flag_is_timed_by_the_completed_set_so_a_late_sample_is_no_contradiction() {
        let mut episodes = flagged_by(&["a", "b"], 41, 42);
        // Turned in at 100, but the completed set only showed 41 and its flagged copy 42 at 117.9.
        episodes.push(session(
            "c",
            SessionSpec {
                initial: vec![8],
                delta_delay: 17.9,
                turn_ins: vec![(100.0, 41)],
                flags: vec![(117.9, 42)],
                ..SessionSpec::default()
            },
        ));
        let catalog = catalog_of([8, 41, 42]);
        let result = infer_co_flags(&population_of(&episodes, &catalog), &PARAMS);

        assert_eq!(pairs(&result), [(41, 42)]);
        assert_eq!((result.edges[0].observations, result.edges[0].contradict), (3, 0));
    }

    #[test]
    fn separate_turn_ins_contradict_even_when_one_delayed_sample_shows_both() {
        let mut episodes = flagged_by(&["a", "b"], 41, 42);
        let mut sampled_together = session(
            "c",
            SessionSpec {
                initial: vec![8],
                turn_ins: vec![(100.0, 41), (110.0, 42)],
                ..SessionSpec::default()
            },
        );
        if let Some(completed) = &mut sampled_together.completed {
            completed.changes = vec![CompletedChange {
                t: 117.9,
                add: Some(vec![41, 42]),
                remove: None,
            }];
        }
        episodes.push(sampled_together);
        let catalog = catalog_of([8, 41, 42]);
        let result = infer_co_flags(&population_of(&episodes, &catalog), &PARAMS);

        assert!(result.edges.is_empty());
        assert_eq!(result.contradicted, 1);
    }

    #[test]
    fn a_breadcrumb_its_target_flags_is_rejected_once_a_character_completes_the_breadcrumb_alone() {
        // Turning in target 42 flags breadcrumb 41, but character c took 41 itself, long before 42.
        let flagged = flagged_by(&["a", "b"], 42, 41);
        let catalog = catalog_of([8, 41, 42]);
        assert_eq!(pairs(&infer_co_flags(&population_of(&flagged, &catalog), &PARAMS)), [(41, 42)]);

        let mut episodes = flagged;
        episodes.push(session(
            "c",
            SessionSpec {
                initial: vec![8],
                turn_ins: vec![(50.0, 41), (400.0, 42)],
                ..SessionSpec::default()
            },
        ));
        assert!(infer_co_flags(&population_of(&episodes, &catalog), &PARAMS).edges.is_empty());
    }

    #[test]
    fn only_quests_seen_landing_in_the_completed_set_during_a_session_are_covered() {
        // 41 lands mid-session for two characters; 42 was always completed before capture started.
        let episodes: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|key| {
                session(
                    key,
                    SessionSpec {
                        initial: vec![8, 42],
                        turn_ins: vec![(100.0, 41)],
                        ..SessionSpec::default()
                    },
                )
            })
            .collect();
        let catalog = catalog_of([8, 41, 42]);

        assert_eq!(infer_co_flags(&population_of(&episodes, &catalog), &PARAMS).covered, [41]);
    }

    #[test]
    fn a_pair_is_rejected_once_a_character_held_both_at_the_same_time() {
        let mut episodes = flagged_by(&["a", "b"], 42, 41);
        episodes.push(session(
            "c",
            SessionSpec {
                initial: vec![8],
                accepts: vec![(10.0, 41), (20.0, 42)],
                turn_ins: vec![(100.0, 42)],
                flags: vec![(100.5, 41)],
                ..SessionSpec::default()
            },
        ));
        let catalog = catalog_of([8, 41, 42]);

        assert!(infer_co_flags(&population_of(&episodes, &catalog), &PARAMS).edges.is_empty());
    }
}
