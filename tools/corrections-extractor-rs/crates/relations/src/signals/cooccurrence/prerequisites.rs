//! Prerequisites from implication: every character that took X had already completed P.
//!
//! A taker of X (held, accepted or completed it) who had not completed P at that moment
//! contradicts "P is a prerequisite of X"; so does one who never completed P at all. Survivors
//! are then filtered on:
//! - support: enough eligible takers, some of them with P observed strictly before X;
//! - lift: P must not be something nearly every comparable character completes anyway
//!   (`null_probability` = chance that all takers had P if X and P were independent);
//! - nearness: if X => P2 => P1, only P2 is X's direct prerequisite. Zone-mates done earlier by
//!   every taker survive that pruning, so the remaining steps are ranked by hand-offs (taking X
//!   within minutes of completing P), which is what tells a direct prerequisite apart.
//!
//! Co-flagged copies ([`super::coflag`]) form one step: any of them will do, and they are never
//! each other's prerequisite. A P that only some takers could take (race/class gated) is a
//! variant: it is emitted only when such variants together cover every taker, as alternatives
//! (P1 OR P2).

use super::coflag::CoFlagged;
use crate::text::to_fixed;
use super::history::{lead_within_session, order, Order, Stamp, When};
use super::population::{Character, Population};
use crate::model::{Evidence, QuestId, RelationField, SessionTime};
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use serde::Serialize;
use std::cmp::Ordering;
use std::sync::OnceLock;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrerequisiteParams {
    /// Takers of X (eligible for P) needed before an edge is judged at all.
    pub min_takers: usize,
    /// Takers with P completed strictly before they took X.
    pub min_ordered: usize,
    /// Largest acceptable chance that all takers had P although X and P are unrelated.
    pub max_null_probability: f64,
    pub order_tolerance_seconds: SessionTime,
    /// Took X at most this long after completing P, in the same session: a hand-off.
    pub handoff_seconds: SessionTime,
}

/// P survived the implication test for X.
#[derive(Clone, Debug)]
pub struct Implication {
    pub quest_id: QuestId,
    pub prereq: QuestId,
    /// Takers of X whose race/class allow P.
    pub takers: usize,
    pub ordered: usize,
    pub unordered: usize,
    /// Seconds from completing P to taking X, for ordered takers who did both in one session,
    /// except those whose P turn-in levelled them up to X's required level: that unlocks X without
    /// any relation (a class quest at level 10 is often taken right after whichever quest gave
    /// level 10).
    pub gaps: Vec<SessionTime>,
    /// Same-session ordered takers who reached X's required level in between.
    pub level_ups: usize,
    /// Cells (race/class, see [`Population::cells`]) of the takers that could take P.
    pub cells: Vec<usize>,
    /// The ordered takers that took X soonest after completing P.
    pub witnesses: Vec<Evidence>,
}

/// Comparable characters who did not take X: how many could take P, and how many completed it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Baseline {
    pub completed: usize,
    pub of: usize,
}

impl Baseline {
    fn smoothed_rate(self) -> f64 {
        (self.completed + 1) as f64 / (self.of + 2) as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Every taker could take P.
    Full,
    /// One of several race/class alternatives.
    Variant,
}

impl Shape {
    pub fn name(self) -> &'static str {
        match self {
            Shape::Full => "full",
            Shape::Variant => "variant",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PrerequisiteEdge {
    pub implication: Implication,
    pub field: RelationField,
    pub all_takers: usize,
    pub baseline: Baseline,
    pub null_probability: f64,
    pub shape: Shape,
    /// Steps emitted for X (an AND group of 1 is preQuestSingle); co-flagged copies are one step.
    pub siblings: usize,
    /// Takers who took X within `handoff_seconds` of completing P.
    pub handoffs: usize,
    /// 1 = the step taken soonest after completion; see [`rank_steps`].
    pub rank: usize,
}

pub struct PrerequisiteResult {
    pub edges: Vec<PrerequisiteEdge>,
    /// Quests with enough takers to be judged.
    pub covered: Vec<QuestId>,
    /// Accepted before pruning, then dropped because a nearer prerequisite implies them.
    pub pruned: usize,
    /// Variants whose alternatives did not cover every taker.
    pub uncovered_variants: usize,
}

/// One judged quest's share of the result.
struct Judged {
    edges: Vec<PrerequisiteEdge>,
    pruned: usize,
    uncovered_variants: usize,
}

pub fn infer_prerequisites(population: &Population, params: &PrerequisiteParams, co_flagged: &CoFlagged) -> PrerequisiteResult {
    let finder = ImplicationFinder::new(population, params);
    let judged: Vec<Option<Judged>> = (0..population.quests.len())
        .into_par_iter()
        .map(|index| judge(&finder, index, co_flagged))
        .collect();

    let mut result = PrerequisiteResult {
        edges: Vec::new(),
        covered: Vec::new(),
        pruned: 0,
        uncovered_variants: 0,
    };
    for (quest, judged) in population.quests.iter().zip(judged) {
        let Some(judged) = judged else { continue };
        result.covered.push(quest.id);
        result.edges.extend(judged.edges);
        result.pruned += judged.pruned;
        result.uncovered_variants += judged.uncovered_variants;
    }
    result
}

/// The prerequisite edges of the quest at `quest_index`, or None when too few characters took it.
fn judge(finder: &ImplicationFinder, quest_index: usize, co_flagged: &CoFlagged) -> Option<Judged> {
    let (population, params) = (finder.population, finder.params);
    let quest = population.quests[quest_index];
    let takers = &population.takers[quest_index];
    if takers.len() < params.min_takers {
        return None;
    }

    let mut accepted: Vec<PrerequisiteEdge> = Vec::new();
    for implication in finder.survivors(quest_index) {
        if implication.takers < params.min_takers || implication.ordered < params.min_ordered {
            continue;
        }
        // A co-flagged copy completes a moment before the flag lands on X, which reads as a hand-off.
        if co_flagged.contains(quest.id, implication.prereq) {
            continue;
        }
        let baseline = baseline_for(population, quest_index, implication.prereq);
        let null_probability = baseline.smoothed_rate().powf(implication.takers as f64);
        if null_probability > params.max_null_probability {
            continue;
        }
        accepted.push(PrerequisiteEdge {
            implication: implication.clone(),
            field: RelationField::PreQuestSingle,
            all_takers: takers.len(),
            baseline,
            null_probability,
            shape: if implication.takers == takers.len() { Shape::Full } else { Shape::Variant },
            siblings: 0,
            handoffs: implication.gaps.iter().filter(|&&gap| gap <= params.handoff_seconds).count(),
            rank: 0,
        });
    }

    // P1 is pruned when nearer steps implying it cover every race/class that needs P1: an
    // Alliance-only P2 => P1 says nothing about Horde takers. Co-flagged copies seem to imply each
    // other (the turned-in copy is stamped a moment before the flagged ones), so they never count.
    let is_nearest: Vec<bool> = accepted
        .iter()
        .map(|edge| {
            let prereq = edge.implication.prereq;
            let impliers: Vec<&PrerequisiteEdge> = accepted
                .iter()
                .filter(|other| {
                    let other_prereq = other.implication.prereq;
                    other_prereq != prereq && !co_flagged.contains(other_prereq, prereq) && finder.implies(other_prereq, prereq)
                })
                .collect();
            let covered: FxHashSet<usize> = impliers.iter().flat_map(|other| other.implication.cells.iter().copied()).collect();
            impliers.is_empty() || !edge.implication.cells.iter().all(|cell| covered.contains(cell))
        })
        .collect();
    let accepted_count = accepted.len();
    let nearest: Vec<PrerequisiteEdge> = accepted.into_iter().zip(is_nearest).filter_map(|(edge, keep)| keep.then_some(edge)).collect();
    let pruned = accepted_count - nearest.len();

    // One logical prerequisite is a group of co-flagged copies: completing any of them will do.
    let all_cells: FxHashSet<usize> = takers.iter().map(|&taker| population.characters[taker].cell).collect();
    let covers_all = |edges: &[PrerequisiteEdge]| {
        let cells: FxHashSet<usize> = edges.iter().flat_map(|edge| edge.implication.cells.iter().copied()).collect();
        all_cells.iter().all(|cell| cells.contains(cell))
    };
    let steps = group_co_flagged(nearest, co_flagged);
    let (emitted, uncovered_variants): (Vec<Vec<PrerequisiteEdge>>, usize) = if steps.iter().any(|step| covers_all(step)) {
        (steps.into_iter().filter(|step| covers_all(step)).collect(), 0)
    } else {
        // Race/class variants of one step (X needs P1 OR P2) must cover every taker between them.
        let variants: Vec<PrerequisiteEdge> = steps.into_iter().flatten().collect();
        if variants.is_empty() {
            (Vec::new(), 0)
        } else if covers_all(&variants) {
            (vec![variants], 0)
        } else {
            (Vec::new(), variants.len())
        }
    };

    // The step taken soonest after completion is the prerequisite (preQuestSingle, its copies and
    // variants as alternatives). Further steps that do not imply each other would make an AND
    // group; they are emitted as preQuestGroup, but are mostly zone-mates and score low.
    let siblings = emitted.len();
    let mut edges = Vec::new();
    for (index, mut step) in rank_steps(emitted).into_iter().enumerate() {
        let field = if index == 0 {
            RelationField::PreQuestSingle
        } else {
            RelationField::PreQuestGroup
        };
        step.sort_by_key(|edge| edge.implication.prereq);
        edges.extend(step.into_iter().map(|edge| PrerequisiteEdge {
            field,
            siblings,
            rank: index + 1,
            ..edge
        }));
    }
    Some(Judged {
        edges,
        pruned,
        uncovered_variants,
    })
}

/// Merges edges whose prerequisites are co-flagged copies into one step each.
fn group_co_flagged(edges: Vec<PrerequisiteEdge>, co_flagged: &CoFlagged) -> Vec<Vec<PrerequisiteEdge>> {
    let mut groups: Vec<Vec<PrerequisiteEdge>> = Vec::new();
    for edge in edges {
        let prereq = edge.implication.prereq;
        let (matching, rest): (Vec<_>, Vec<_>) = groups
            .into_iter()
            .partition(|group| group.iter().any(|other| co_flagged.contains(other.implication.prereq, prereq)));
        groups = rest;
        let mut merged = vec![edge];
        merged.extend(matching.into_iter().flatten());
        groups.push(merged);
    }
    groups
}

/// Orders steps so the likeliest direct prerequisite comes first, by each step's best member: most
/// hand-offs per taker, then the shortest median lead time, then the most ordered observations. A
/// zone-mate every taker happened to finish earlier has long, scattered lead times.
fn rank_steps(mut steps: Vec<Vec<PrerequisiteEdge>>) -> Vec<Vec<PrerequisiteEdge>> {
    fn handoff_rate(edge: &PrerequisiteEdge) -> f64 {
        edge.handoffs as f64 / edge.implication.takers as f64
    }
    /// The upper median; infinite without any gap.
    fn median(values: &[SessionTime]) -> SessionTime {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        sorted.get(sorted.len() / 2).copied().unwrap_or(SessionTime::INFINITY)
    }
    fn better(a: &PrerequisiteEdge, b: &PrerequisiteEdge) -> Ordering {
        handoff_rate(b)
            .total_cmp(&handoff_rate(a))
            .then_with(|| median(&a.implication.gaps).total_cmp(&median(&b.implication.gaps)))
            .then_with(|| b.implication.ordered.cmp(&a.implication.ordered))
            .then_with(|| a.implication.prereq.cmp(&b.implication.prereq))
    }
    fn best(step: &[PrerequisiteEdge]) -> &PrerequisiteEdge {
        step.iter().min_by(|a, b| better(a, b)).expect("steps are never empty")
    }
    steps.sort_by(|a, b| better(best(a), best(b)));
    steps
}

/// How common P is among characters at comparable progress who did not take X, taking the more
/// common of two populations: those who could take X (the right comparison, but class-gated
/// quests leave only a handful) and everyone who could take P at X's level. Overstating how common
/// P is only costs recall.
fn baseline_for(population: &Population, quest_index: usize, prereq: QuestId) -> Baseline {
    let quest = population.quests[quest_index];
    let prereq_index = population.index_of(prereq).expect("implications name quests of the population");
    let mut narrow = Baseline::default();
    let mut broad = Baseline::default();
    for character in &population.characters {
        let history = &character.history;
        if history.taken.contains_key(&quest.id) || !population.allows(character, prereq_index) {
            continue;
        }
        if let (Some(required), Some(level)) = (quest.required_level, history.max_level) {
            if level < required {
                continue;
            }
        }
        let completed = usize::from(history.done.contains_key(&prereq));
        broad.of += 1;
        broad.completed += completed;
        if population.could_take(character, quest_index) {
            narrow.of += 1;
            narrow.completed += completed;
        }
    }
    if narrow.smoothed_rate() >= broad.smoothed_rate() {
        narrow
    } else {
        broad
    }
}

/// Computes, once per quest, the implications that survive for it.
struct ImplicationFinder<'p, 'e> {
    population: &'p Population<'e>,
    params: &'p PrerequisiteParams,
    /// Per quest index, its surviving implications by ascending prerequisite id.
    survivors: Vec<OnceLock<Vec<Implication>>>,
}

impl<'p, 'e> ImplicationFinder<'p, 'e> {
    fn new(population: &'p Population<'e>, params: &'p PrerequisiteParams) -> Self {
        ImplicationFinder {
            population,
            params,
            survivors: population.quests.iter().map(|_| OnceLock::new()).collect(),
        }
    }

    /// `later` => `earlier` with at least one character showing the order (used to prune
    /// transitive edges).
    fn implies(&self, later: QuestId, earlier: QuestId) -> bool {
        let Some(index) = self.population.index_of(later) else { return false };
        let survivors = self.survivors(index);
        survivors
            .binary_search_by_key(&earlier, |implication| implication.prereq)
            .is_ok_and(|found| survivors[found].ordered > 0)
    }

    fn survivors(&self, quest_index: usize) -> &[Implication] {
        self.survivors[quest_index].get_or_init(|| self.compute_survivors(quest_index))
    }

    fn compute_survivors(&self, quest_index: usize) -> Vec<Implication> {
        let population = self.population;
        let takers = &population.takers[quest_index];
        // The takers by race/class cell, cells in the order their first taker appears.
        let mut cells: Vec<(usize, Vec<&Character>)> = Vec::new();
        for &taker in takers {
            let character = &population.characters[taker];
            match cells.iter_mut().find(|(cell, _)| *cell == character.cell) {
                Some((_, members)) => members.push(character),
                None => cells.push((character.cell, vec![character])),
            }
        }

        // How many takers completed each quest their race/class allows.
        let mut counts = vec![0usize; population.quests.len()];
        let mut touched = Vec::new();
        for (_, members) in &cells {
            for character in members {
                for &index in &character.done {
                    if index == quest_index || !population.allows(character, index) {
                        continue;
                    }
                    if counts[index] == 0 {
                        touched.push(index);
                    }
                    counts[index] += 1;
                }
            }
        }

        let mut survivors = Vec::new();
        for index in touched {
            let eligible: usize = cells
                .iter()
                .filter(|(cell, _)| population.cells[*cell].allows[index])
                .map(|(_, members)| members.len())
                .sum();
            // An eligible taker who never completed P contradicts it; measured, tolerating even 2% of
            // takers lets more zone-mates through than it rescues prerequisites.
            if counts[index] < eligible {
                continue;
            }
            if let Some(implication) = self.check_order(quest_index, index, &cells) {
                survivors.push(implication);
            }
        }
        survivors.sort_by_key(|implication| implication.prereq);
        survivors
    }

    /// None as soon as one taker took X before completing P.
    fn check_order(&self, quest_index: usize, prereq_index: usize, cells: &[(usize, Vec<&Character>)]) -> Option<Implication> {
        let population = self.population;
        let quest = population.quests[quest_index];
        let prereq = population.quests[prereq_index].id;
        let mut implication = Implication {
            quest_id: quest.id,
            prereq,
            takers: 0,
            ordered: 0,
            unordered: 0,
            gaps: Vec::new(),
            level_ups: 0,
            cells: Vec::new(),
            witnesses: Vec::new(),
        };
        struct Witness<'c, 'e> {
            lead: Option<SessionTime>,
            character: &'c Character<'e>,
            done: Stamp,
            taken: Stamp,
        }
        let mut witnesses: Vec<Witness> = Vec::new();
        for (cell, members) in cells {
            if !population.cells[*cell].allows[prereq_index] {
                continue;
            }
            implication.cells.push(*cell);
            for &character in members {
                implication.takers += 1;
                let taken = character.history.taken[&quest.id];
                let done = character.history.done[&prereq];
                match order(&done, &taken, self.params.order_tolerance_seconds) {
                    Order::After => return None,
                    Order::Unordered => {
                        implication.unordered += 1;
                        continue;
                    }
                    Order::Before => implication.ordered += 1,
                }
                let lead = lead_within_session(&done, &taken);
                if let Some(lead) = lead {
                    if reached_level(character, &done, &taken, quest.required_level) {
                        implication.level_ups += 1;
                    } else {
                        implication.gaps.push(lead);
                    }
                }
                witnesses.push(Witness { lead, character, done, taken });
            }
        }
        // Stable: equal leads (and every taker without one) keep cell, then character order.
        witnesses.sort_by(|a, b| a.lead.unwrap_or(SessionTime::INFINITY).total_cmp(&b.lead.unwrap_or(SessionTime::INFINITY)));
        implication.witnesses = witnesses
            .iter()
            .take(5)
            .map(|entry| witness(entry.character, prereq, quest.id, &entry.done, &entry.taken))
            .collect();
        Some(implication)
    }
}

const LEVEL_UP_LEAD_SECONDS: SessionTime = 5.0;

/// The character went from below `level` to at least `level` around `from` and before `to`, in
/// one session.
fn reached_level(character: &Character, from: &Stamp, to: &Stamp, level: Option<i64>) -> bool {
    let (Some(level), Some(from_t), Some(to_t)) = (level, from.time(), to.time()) else {
        return false;
    };
    let timeline = &character.history.sessions[from.session];
    // The level-up is often logged a moment before the turn-in event that caused it.
    match (timeline.level_at(from_t - LEVEL_UP_LEAD_SECONDS), timeline.level_at(to_t)) {
        (Some(before), Some(after)) => before < level && after >= level,
        _ => false,
    }
}

fn describe(character: &Character, stamp: &Stamp) -> String {
    let episode = character.history.episode_key(stamp);
    match stamp.when {
        When::AtStart => format!("before session {episode} started"),
        When::During { t, .. } => format!("at {episode} t={}", to_fixed(t, 1)),
    }
}

fn witness(character: &Character, prereq: QuestId, quest_id: QuestId, done: &Stamp, taken: &Stamp) -> Evidence {
    Evidence {
        reference: character.history.episode_key(taken).to_owned(),
        t: taken.time(),
        text: format!(
            "character {}: completed {prereq} {}, took {quest_id} {}",
            character.history.key,
            describe(character, done),
            describe(character, taken)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CatalogQuest, QuestEpisode};
    use crate::signals::cooccurrence::fixtures::{population_of, session, SessionSpec};
    use crate::test_support::{catalog, catalog_quest};

    const PARAMS: PrerequisiteParams = PrerequisiteParams {
        min_takers: 3,
        min_ordered: 1,
        max_null_probability: 0.05,
        order_tolerance_seconds: 10.0,
        handoff_seconds: 30.0,
    };

    /// The edges for `quest_id` as (prereq, field, rank, shape).
    fn edges_for(result: &PrerequisiteResult, quest_id: QuestId) -> Vec<(QuestId, RelationField, usize, Shape)> {
        result
            .edges
            .iter()
            .filter(|edge| edge.implication.quest_id == quest_id)
            .map(|edge| (edge.implication.prereq, edge.field, edge.rank, edge.shape))
            .collect()
    }

    fn infer(episodes: &[QuestEpisode], quests: Vec<CatalogQuest>, co_flagged: &CoFlagged) -> PrerequisiteResult {
        let catalog = catalog(quests);
        infer_prerequisites(&population_of(episodes, &catalog), &PARAMS, co_flagged)
    }

    /// Characters who did nothing relevant: the baseline that makes a prerequisite non-universal.
    fn bystanders(count: usize, race: i64) -> Vec<QuestEpisode> {
        (0..count)
            .map(|i| {
                session(
                    &format!("bystander-{race}-{i}"),
                    SessionSpec {
                        race,
                        level: 10,
                        initial: vec![8],
                        ..SessionSpec::default()
                    },
                )
            })
            .collect()
    }

    /// Characters `keys` of `race` who turn in `prereq` at 100 and accept `quest_id` at 101.
    fn handed_off(keys: &[&str], race: i64, prereq: QuestId, quest_id: QuestId) -> Vec<QuestEpisode> {
        keys.iter()
            .map(|key| {
                session(
                    key,
                    SessionSpec {
                        race,
                        initial: vec![8],
                        turn_ins: vec![(100.0, prereq)],
                        accepts: vec![(101.0, quest_id)],
                        ..SessionSpec::default()
                    },
                )
            })
            .collect()
    }

    fn quests(ids: &[QuestId]) -> Vec<CatalogQuest> {
        ids.iter().map(|&id| catalog_quest(id)).collect()
    }

    const ALLIANCE_ONLY: u64 = 4_294_967_373;
    const HORDE_ONLY: u64 = 8_589_934_770;

    #[test]
    fn the_hand_off_predecessor_is_emitted_the_transitive_one_pruned_and_a_zone_mate_ranked_below() {
        // 1 -> 2 -> 3, and every taker of 3 also finished zone-mate 9 while holding 2.
        let mut episodes: Vec<QuestEpisode> = ["a", "b", "c", "d"]
            .iter()
            .map(|key| {
                session(
                    key,
                    SessionSpec {
                        initial: vec![1],
                        accepts: vec![(10.0, 2), (101.0, 3)],
                        turn_ins: vec![(50.0, 9), (100.0, 2), (200.0, 3)],
                        ..SessionSpec::default()
                    },
                )
            })
            .collect();
        episodes.extend(bystanders(3, 1));
        let result = infer(&episodes, quests(&[1, 2, 3, 8, 9]), &CoFlagged::default());

        assert_eq!(
            edges_for(&result, 3),
            [
                (2, RelationField::PreQuestSingle, 1, Shape::Full),
                (9, RelationField::PreQuestGroup, 2, Shape::Full)
            ]
        );
        assert_eq!(result.pruned, 1, "1 is implied by 2");
    }

    #[test]
    fn a_prerequisite_is_dropped_as_soon_as_one_character_held_the_quest_before_completing_it() {
        let mut episodes = handed_off(&["a", "b", "c"], 1, 2, 3);
        episodes.extend(bystanders(3, 1));
        let result = infer(&episodes, quests(&[2, 3, 8]), &CoFlagged::default());
        assert_eq!(edges_for(&result, 3), [(2, RelationField::PreQuestSingle, 1, Shape::Full)]);

        episodes.push(session(
            "d",
            SessionSpec {
                initial: vec![8],
                accepts: vec![(10.0, 3)],
                turn_ins: vec![(100.0, 2)],
                ..SessionSpec::default()
            },
        ));
        assert_eq!(edges_for(&infer(&episodes, quests(&[2, 3, 8]), &CoFlagged::default()), 3), []);
    }

    #[test]
    fn a_quest_nearly_every_comparable_character_completed_anyway_is_no_prerequisite() {
        // Everyone did 7, takers of 3 included: no lift, however consistent the order.
        let takers = handed_off(&["a", "b", "c"], 1, 7, 3);
        let did_seven = ["d", "e", "f", "g", "h", "i"].iter().map(|key| {
            session(
                key,
                SessionSpec {
                    initial: vec![8, 7],
                    ..SessionSpec::default()
                },
            )
        });
        let with_seven: Vec<QuestEpisode> = takers.iter().cloned().chain(did_seven).collect();
        assert_eq!(edges_for(&infer(&with_seven, quests(&[3, 7, 8]), &CoFlagged::default()), 3), []);

        let without_seven: Vec<QuestEpisode> = takers.into_iter().chain(bystanders(6, 1)).collect();
        assert_eq!(
            edges_for(&infer(&without_seven, quests(&[3, 7, 8]), &CoFlagged::default()), 3),
            [(7, RelationField::PreQuestSingle, 1, Shape::Full)]
        );
    }

    #[test]
    fn faction_variants_are_emitted_as_alternatives_when_together_they_cover_every_taker() {
        let catalog = vec![
            catalog_quest(30),
            CatalogQuest {
                required_races: Some(ALLIANCE_ONLY),
                ..catalog_quest(31)
            },
            CatalogQuest {
                required_races: Some(HORDE_ONLY),
                ..catalog_quest(32)
            },
            catalog_quest(8),
        ];
        let mut episodes = handed_off(&["a", "b", "c"], 1, 31, 30);
        episodes.extend(handed_off(&["d", "e", "f"], 2, 32, 30));
        episodes.extend(bystanders(2, 1));
        episodes.extend(bystanders(2, 2));

        assert_eq!(
            edges_for(&infer(&episodes, catalog, &CoFlagged::default()), 30),
            [
                (31, RelationField::PreQuestSingle, 1, Shape::Variant),
                (32, RelationField::PreQuestSingle, 1, Shape::Variant)
            ]
        );
    }

    #[test]
    fn a_hand_off_the_prerequisites_level_up_explains_is_not_counted() {
        // Turning in 2 dings level 10, which unlocks 3 on its own.
        let catalog = vec![
            catalog_quest(2),
            catalog_quest(8),
            CatalogQuest {
                required_level: Some(10),
                ..catalog_quest(3)
            },
        ];
        let mut episodes: Vec<QuestEpisode> = ["a", "b", "c"]
            .iter()
            .map(|key| {
                session(
                    key,
                    SessionSpec {
                        level: 9,
                        initial: vec![8],
                        turn_ins: vec![(100.0, 2)],
                        level_ups: vec![(100.2, 10)],
                        accepts: vec![(101.0, 3)],
                        ..SessionSpec::default()
                    },
                )
            })
            .collect();
        episodes.extend(bystanders(3, 1));
        let result = infer(&episodes, catalog, &CoFlagged::default());
        let edge = result.edges.iter().find(|edge| edge.implication.quest_id == 3).expect("3 has an edge");

        assert_eq!(
            (edge.implication.prereq, edge.implication.ordered, edge.handoffs, edge.implication.level_ups),
            (2, 3, 0, 3)
        );
    }

    #[test]
    fn a_faction_only_step_does_not_prune_a_prerequisite_the_other_faction_needs() {
        // Alliance goes 5 -> 6 -> 30, Horde goes 5 -> 30: the Alliance-only 6 must not prune 5.
        let catalog = vec![
            catalog_quest(30),
            catalog_quest(5),
            CatalogQuest {
                required_races: Some(ALLIANCE_ONLY),
                ..catalog_quest(6)
            },
            catalog_quest(8),
        ];
        let mut episodes: Vec<QuestEpisode> = ["a", "b", "c"]
            .iter()
            .map(|key| {
                session(
                    key,
                    SessionSpec {
                        race: 1,
                        initial: vec![8],
                        turn_ins: vec![(50.0, 5), (100.0, 6)],
                        accepts: vec![(60.0, 6), (101.0, 30)],
                        ..SessionSpec::default()
                    },
                )
            })
            .collect();
        episodes.extend(handed_off(&["d", "e", "f"], 2, 5, 30));
        episodes.extend(bystanders(2, 1));
        episodes.extend(bystanders(2, 2));

        assert!(edges_for(&infer(&episodes, catalog, &CoFlagged::default()), 30).contains(&(5, RelationField::PreQuestSingle, 1, Shape::Full)));
    }

    #[test]
    fn co_flagged_copies_are_one_step_and_never_each_others_prerequisite() {
        // Turning in 41 flags its copy 42 half a second later, then 43 is handed off.
        let mut episodes: Vec<QuestEpisode> = ["a", "b", "c"]
            .iter()
            .map(|key| {
                session(
                    key,
                    SessionSpec {
                        initial: vec![8],
                        turn_ins: vec![(100.0, 41)],
                        flags: vec![(100.5, 42)],
                        accepts: vec![(102.0, 43)],
                        ..SessionSpec::default()
                    },
                )
            })
            .collect();
        episodes.extend(bystanders(3, 1));
        let co_flagged = CoFlagged::new([(42, 41)]);
        let result = infer(&episodes, quests(&[8, 41, 42, 43]), &co_flagged);

        let steps: Vec<(QuestId, usize)> = edges_for(&result, 43).into_iter().map(|(prereq, _, rank, _)| (prereq, rank)).collect();
        assert_eq!(steps, [(41, 1), (42, 1)]);
        assert_eq!(edges_for(&result, 42), []);
        let unaware = infer(&episodes, quests(&[8, 41, 42, 43]), &CoFlagged::default());
        let prereqs: Vec<QuestId> = edges_for(&unaware, 42).into_iter().map(|(prereq, ..)| prereq).collect();
        assert_eq!(prereqs, [41]);
    }
}
