//! Turns per-episode observations into scored candidates: each kind of evidence is counted per
//! independent character, a pair must be both optional and dependent (see [`super::observe`]),
//! and each breadcrumb keeps only its best target because breadcrumbForQuestId is a single quest.

use super::links::{FxIndexMap, Pair};
use super::observe::{Observation, ObservationKind};
use crate::model::{Evidence, QuestCatalog, QuestId, RelationCandidate, RelationField};
use rustc_hash::FxHashSet;
use std::collections::{BTreeMap, BTreeSet};

// Weights are calibrated against Questie's Classic and Forever breadcrumbs (score command), so a
// score roughly reads as the chance the edge is right.

/// How strongly one character's observation of each dependent kind says B and T gate each other.
/// The server dropping B as T is accepted is the breadcrumb mechanism itself. Chains also auto-pop
/// T or reveal it on B's turn-in, but optionality already rules chains out. B missing once T is
/// done is weakest, because B's own prerequisites hide it just as well.
fn dependent_weight(kind: ObservationKind) -> f64 {
    match kind {
        ObservationKind::ServerDrop => 0.95,
        ObservationKind::Reveal => 0.8,
        ObservationKind::AutoPop => 0.75,
        ObservationKind::BlockB => 0.5,
        _ => 0.0,
    }
}

/// Characters offered B while T was done, or T while B was in the log. Real breadcrumbs almost
/// never show either (Forever blocks both ways), so one is treated as noise and more rule the
/// pair out.
const MAX_SHOWN: usize = 1;
const SHOWN_PENALTY: f64 = 0.25;

/// One character's "removed" could still be a manual abandon ingest could not tell apart.
const MIN_SERVER_DROP_CHARACTERS: usize = 2;

/// Characters needed on a pair before its quests count as covered (for recall).
pub const MIN_COVERAGE_CHARACTERS: usize = 2;

/// Everything the episodes said about one pair.
pub struct PairTally<'e> {
    pub pair: Pair,
    /// Characters per observation kind.
    pub characters: BTreeMap<ObservationKind, FxHashSet<&'e str>>,
    /// The first observation of each kind, as the example a reviewer sees.
    pub examples: BTreeMap<ObservationKind, Evidence>,
}

impl PairTally<'_> {
    fn count(&self, kind: ObservationKind) -> usize {
        self.characters.get(&kind).map_or(0, FxHashSet::len)
    }

    /// Distinct characters with an observation of any of `kinds`.
    fn characters_of(&self, kinds: impl IntoIterator<Item = ObservationKind>) -> usize {
        let characters: FxHashSet<&str> = kinds.into_iter().flat_map(|kind| self.characters.get(&kind)).flatten().copied().collect();
        characters.len()
    }
}

/// Observations of every episode, per pair, in the order pairs were first observed (which decides
/// ties further on).
#[derive(Default)]
pub struct Tally<'e> {
    pub pairs: FxIndexMap<Pair, PairTally<'e>>,
}

impl<'e> Tally<'e> {
    pub fn add(&mut self, character_key: &'e str, observations: Vec<Observation>) {
        for Observation { pair, kind, evidence } in observations {
            let tally = self.pairs.entry(pair).or_insert_with(|| PairTally {
                pair,
                characters: BTreeMap::new(),
                examples: BTreeMap::new(),
            });
            tally.characters.entry(kind).or_default().insert(character_key);
            tally.examples.entry(kind).or_insert(evidence);
        }
    }
}

pub struct PairJudgement {
    pub pair: Pair,
    pub score: f64,
    pub support: u32,
    pub contradict: u32,
    /// Characters per kind, for every kind any character observed.
    pub counts: BTreeMap<ObservationKind, u32>,
    pub evidence: Vec<Evidence>,
}

/// None unless the pair is both optional and dependent and the client did not deny it.
pub fn judge_pair(pair: &PairTally) -> Option<PairJudgement> {
    let count = |kind| pair.count(kind);
    let flagged = count(ObservationKind::Flagged);
    if count(ObservationKind::NotFlagged) > flagged {
        return None;
    }
    let free = count(ObservationKind::Free);
    let dependent_kinds: Vec<ObservationKind> = ObservationKind::DEPENDENT
        .into_iter()
        .filter(|&kind| kind != ObservationKind::ServerDrop || count(kind) >= MIN_SERVER_DROP_CHARACTERS)
        .collect();
    if (free == 0 && flagged == 0) || pair.characters_of(dependent_kinds.iter().copied()) == 0 {
        return None;
    }
    let shown = pair.characters_of([ObservationKind::ShowB, ObservationKind::ShowT]);
    if shown > MAX_SHOWN {
        return None;
    }

    // Chains rarely leak even one free character, so one already says a lot; the client flag settles it.
    let free_optional = if free > 0 { 1.0 - 0.2 * 0.4_f64.powi(free as i32 - 1) } else { 0.0 };
    let optional = free_optional.max(if flagged > 0 { 0.9 } else { 0.0 });
    let doubt: f64 = dependent_kinds
        .iter()
        .map(|&kind| (1.0 - dependent_weight(kind)).powi(count(kind).min(2) as i32))
        .product();
    // A character that only got T after completing B (a chain) outweighs two that got it freely.
    let unlock = count(ObservationKind::Unlock);
    let not_chain = if free + unlock == 0 { 1.0 } else { free as f64 / (free + 2 * unlock) as f64 };
    let score = optional * (1.0 - doubt) * not_chain * if shown > 0 { SHOWN_PENALTY } else { 1.0 };

    let evidence = ObservationKind::ALL
        .into_iter()
        .filter_map(|kind| {
            pair.examples.get(&kind).map(|example| Evidence {
                text: format!("{}: {}", kind.name(), example.text),
                ..example.clone()
            })
        })
        .take(5)
        .collect();
    Some(PairJudgement {
        pair: pair.pair,
        score: (score * 1000.0).round() / 1000.0,
        support: pair.characters_of(ObservationKind::OPTIONAL.into_iter().chain(dependent_kinds)) as u32,
        contradict: pair.characters_of(ObservationKind::CONTRADICTING) as u32,
        counts: pair.characters.iter().map(|(&kind, characters)| (kind, characters.len() as u32)).collect(),
        evidence,
    })
}

fn quest_name(catalog: &QuestCatalog, quest_id: QuestId) -> String {
    match catalog.quest(quest_id).and_then(|quest| quest.name.as_deref()).filter(|name| !name.is_empty()) {
        Some(name) => format!("{quest_id} \"{name}\""),
        None => quest_id.to_string(),
    }
}

/// Both Questie fields for each judged pair, keeping one target per breadcrumb. T's follow-ups at
/// the same giver inherit "B hidden once done" and often tie with T; more characters reach T than
/// its follow-ups, so support breaks the tie toward the start of the chain.
pub fn to_candidates(judgements: &[PairJudgement], catalog: &QuestCatalog) -> Vec<RelationCandidate> {
    let mut by_breadcrumb: FxIndexMap<QuestId, Vec<&PairJudgement>> = FxIndexMap::default();
    for judgement in judgements {
        by_breadcrumb.entry(judgement.pair.breadcrumb).or_default().push(judgement);
    }

    let mut candidates = Vec::new();
    for mut options in by_breadcrumb.into_values() {
        options.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.support.cmp(&a.support)));
        let (winner, others) = options.split_first().expect("every breadcrumb has a judgement");
        let Pair { breadcrumb, target } = winner.pair;
        let counts: Vec<String> = winner.counts.iter().map(|(kind, n)| format!("{} {n}", kind.name())).collect();
        let alternatives = if others.is_empty() {
            String::new()
        } else {
            let others: Vec<String> = others.iter().map(|other| format!("{} ({})", other.pair.target, other.score)).collect();
            format!("; other targets {}", others.join(", "))
        };
        let note = format!(
            "{} -> {}; characters: {}{alternatives}",
            quest_name(catalog, breadcrumb),
            quest_name(catalog, target),
            counts.join(", ")
        );
        let candidate = |quest_id, field, target| RelationCandidate {
            quest_id,
            field,
            target,
            score: winner.score,
            support: winner.support,
            contradict: winner.contradict,
            evidence: winner.evidence.clone(),
            note: Some(note.clone()),
        };
        candidates.push(candidate(breadcrumb, RelationField::BreadcrumbForQuestId, target));
        candidates.push(candidate(target, RelationField::Breadcrumbs, breadcrumb));
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.quest_id.cmp(&b.quest_id)));
    candidates
}

/// Quests on either side of a pair that enough characters had something to say about.
pub fn covered_quests(tally: &Tally, min_characters: usize) -> Vec<QuestId> {
    let covered: BTreeSet<QuestId> = tally
        .pairs
        .values()
        .filter(|pair| pair.characters_of(ObservationKind::ALL) >= min_characters)
        .flat_map(|pair| [pair.pair.breadcrumb, pair.pair.target])
        .collect();
    covered.into_iter().collect()
}

pub struct BreadcrumbResult<'e> {
    pub tally: Tally<'e>,
    /// Judged pairs in the order they were first observed.
    pub judgements: Vec<PairJudgement>,
    pub candidates: Vec<RelationCandidate>,
    pub covered_quest_ids: Vec<QuestId>,
}

pub fn judge_tally<'e>(tally: Tally<'e>, catalog: &QuestCatalog) -> BreadcrumbResult<'e> {
    let judgements: Vec<PairJudgement> = tally.pairs.values().filter_map(judge_pair).collect();
    BreadcrumbResult {
        candidates: to_candidates(&judgements, catalog),
        covered_quest_ids: covered_quests(&tally, MIN_COVERAGE_CHARACTERS),
        judgements,
        tally,
    }
}
