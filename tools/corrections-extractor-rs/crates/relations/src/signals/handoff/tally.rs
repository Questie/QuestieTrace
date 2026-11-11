//! Folds per-episode observations into per-pair counts of independent characters. One character
//! contributes at most once to each count of a pair, however many sessions or repeats it has.

use super::counter::{CounterKind, CounterObservation};
use super::detect::{HandoffKind, HandoffObservation};
use crate::model::{Evidence, QuestEpisode, QuestId};
use crate::text::to_fixed;
use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};

/// P -> X: turning in `prerequisite` was followed by `quest` being offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pair {
    pub prerequisite: QuestId,
    pub quest: QuestId,
}

/// Characters by their pseudonymous character key.
pub type Characters<'e> = FxHashSet<&'e str>;

/// For another observed prerequisite Q of X: the characters for whom Q was already completed, or
/// not, when they turned in P and got X.
#[derive(Default)]
pub struct CompanionStates<'e> {
    pub done: Characters<'e>,
    pub not_done: Characters<'e>,
}

/// Per (P, X) pair: the characters behind each kind of observation.
pub struct PairStats<'e> {
    pub pair: Pair,
    pub pop: Characters<'e>,
    pub pop_confounded: Characters<'e>,
    pub list: Characters<'e>,
    pub list_confounded: Characters<'e>,
    pub not_started: Characters<'e>,
    pub in_log: Characters<'e>,
    pub alternative: Characters<'e>,
    /// Per other observed prerequisite of X; tells ALL-of groups from ANY-of lists.
    pub companions: FxHashMap<QuestId, CompanionStates<'e>>,
    /// Supporting examples in the order they were met; lower priority is shown first.
    evidence: Vec<(EvidencePriority, Evidence)>,
    counter_evidence: Vec<Evidence>,
}

/// Clean pops first, then confounded pops, clean lists, confounded lists.
type EvidencePriority = u8;

const MAX_EVIDENCE: usize = 5;
const MAX_COUNTER_EVIDENCE: usize = 2;

impl<'e> PairStats<'e> {
    fn new(pair: Pair) -> Self {
        PairStats {
            pair,
            pop: Characters::default(),
            pop_confounded: Characters::default(),
            list: Characters::default(),
            list_confounded: Characters::default(),
            not_started: Characters::default(),
            in_log: Characters::default(),
            alternative: Characters::default(),
            companions: FxHashMap::default(),
            evidence: Vec::new(),
            counter_evidence: Vec::new(),
        }
    }

    /// Some character was offered X while P was neither completed nor explained by an alternative.
    pub fn is_contradicted(&self) -> bool {
        !self.not_started.is_empty() || !self.in_log.is_empty()
    }

    /// Characters with a pop, clean or not.
    pub fn pop_characters(&self) -> Characters<'e> {
        self.pop.union(&self.pop_confounded).copied().collect()
    }

    /// Characters with any supporting observation.
    pub fn support(&self) -> usize {
        let lists = self.list.iter().chain(&self.list_confounded);
        self.pop.iter().chain(&self.pop_confounded).chain(lists).copied().collect::<Characters>().len()
    }

    /// Up to five examples, pops first, keeping room for a couple of counter-examples.
    pub fn pick_evidence(&self) -> Vec<Evidence> {
        let mut support: Vec<&(EvidencePriority, Evidence)> = self.evidence.iter().collect();
        support.sort_by_key(|(priority, _)| *priority);
        let counter = &self.counter_evidence[..self.counter_evidence.len().min(MAX_COUNTER_EVIDENCE)];
        support
            .into_iter()
            .take(MAX_EVIDENCE - counter.len())
            .map(|(_, evidence)| evidence.clone())
            .chain(counter.iter().cloned())
            .collect()
    }

    pub fn counts_note(&self) -> String {
        format!(
            "pop {}+{}?, list {}+{}?, against {} not started / {} in log, {} via alternatives",
            self.pop.len(),
            self.pop_confounded.len(),
            self.list.len(),
            self.list_confounded.len(),
            self.not_started.len(),
            self.in_log.len(),
            self.alternative.len()
        )
    }
}

/// Every pair, in the order it was first met: observed pairs first, then pairs added for
/// same-name copies.
#[derive(Default)]
pub struct HandoffTally<'e> {
    pairs: IndexMap<Pair, PairStats<'e>, FxBuildHasher>,
}

impl<'e> HandoffTally<'e> {
    /// The pair's stats, created empty if needed (variant edges are tracked for counter-evidence only).
    pub fn stats_for(&mut self, pair: Pair) -> &mut PairStats<'e> {
        self.pairs.entry(pair).or_insert_with(|| PairStats::new(pair))
    }

    pub fn get(&self, pair: Pair) -> Option<&PairStats<'e>> {
        self.pairs.get(&pair)
    }

    /// In the order the pairs were first met.
    pub fn pairs(&self) -> impl ExactSizeIterator<Item = &PairStats<'e>> {
        self.pairs.values()
    }

    pub fn add_handoff(&mut self, episode: &'e QuestEpisode, observation: &HandoffObservation) {
        let stats = self.stats_for(Pair {
            prerequisite: observation.turned_in,
            quest: observation.offered,
        });
        let clean = observation.confounders.is_empty();
        let (bucket, priority) = match (observation.kind, clean) {
            (HandoffKind::Pop, true) => (&mut stats.pop, 0),
            (HandoffKind::Pop, false) => (&mut stats.pop_confounded, 1),
            (HandoffKind::List { .. }, true) => (&mut stats.list, 2),
            (HandoffKind::List { .. }, false) => (&mut stats.list_confounded, 3),
        };
        if !bucket.insert(&episode.character_key) {
            return;
        }
        let evidence = Evidence {
            reference: episode.key.clone(),
            t: Some(observation.turn_in_at),
            text: describe_handoff(observation),
        };
        stats.evidence.push((priority, evidence));
    }

    /// Records, for each of `states`' quests, whether it was completed when the character turned
    /// in the pair's prerequisite and got its quest.
    pub fn add_companion_states(&mut self, episode: &'e QuestEpisode, pair: Pair, states: impl IntoIterator<Item = (QuestId, bool)>) {
        let Some(stats) = self.pairs.get_mut(&pair) else { return };
        for (quest_id, completed) in states {
            let entry = stats.companions.entry(quest_id).or_default();
            let bucket = if completed { &mut entry.done } else { &mut entry.not_done };
            bucket.insert(&episode.character_key);
        }
    }

    pub fn add_counter(&mut self, episode: &'e QuestEpisode, observation: &CounterObservation) {
        let Some(stats) = self.pairs.get_mut(&observation.pair) else { return };
        let bucket = match observation.kind {
            CounterKind::NotStarted => &mut stats.not_started,
            CounterKind::InLog => &mut stats.in_log,
            CounterKind::Alternative => &mut stats.alternative,
        };
        if !bucket.insert(&episode.character_key) {
            return;
        }
        if observation.kind != CounterKind::Alternative {
            stats.counter_evidence.push(Evidence {
                reference: episode.key.clone(),
                t: Some(observation.t),
                text: describe_counter(observation),
            });
        }
    }
}

fn describe_handoff(observation: &HandoffObservation) -> String {
    let HandoffObservation {
        turned_in,
        offered,
        giver,
        turn_in_at,
        offered_at,
        ..
    } = observation;
    let giver = giver.label();
    let delay = to_fixed(offered_at - turn_in_at, 1);
    let base = match observation.kind {
        HandoffKind::Pop => format!("turned in {turned_in} at {giver}, its accept dialog for {offered} opened {delay}s later"),
        HandoffKind::List { previous_list_at } => format!(
            "turned in {turned_in} at {giver}; its list {delay}s later offered {offered}, its list {}s before did not",
            to_fixed(turn_in_at - previous_list_at, 1)
        ),
    };
    if observation.confounders.is_empty() {
        return base;
    }
    let shown: Vec<String> = observation.confounders.iter().take(3).map(ToString::to_string).collect();
    format!("{base} (also: {})", shown.join(", "))
}

fn describe_counter(observation: &CounterObservation) -> String {
    let place = observation.giver.map(|giver| format!(" by {}", giver.label())).unwrap_or_default();
    let state = match observation.kind {
        CounterKind::InLog => "in the log, not completed",
        CounterKind::NotStarted | CounterKind::Alternative => "not started",
    };
    let Pair { prerequisite, quest } = observation.pair;
    format!("against: offered {quest}{place} while {prerequisite} was {state}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::QuestEpisode;
    use crate::signals::handoff::detect::Confounder;
    use crate::test_support::{make_episode, npc};

    fn observation(kind: HandoffKind, confounders: Vec<Confounder>) -> HandoffObservation {
        HandoffObservation {
            kind,
            turned_in: 1,
            offered: 2,
            giver: npc(7),
            turn_in_at: 10.0,
            offered_at: 10.4,
            confounders,
        }
    }

    fn counter(kind: CounterKind) -> CounterObservation {
        CounterObservation {
            kind,
            pair: Pair { prerequisite: 1, quest: 2 },
            t: 3.0,
            giver: None,
        }
    }

    #[test]
    fn evidence_shows_pops_before_lists_and_keeps_room_for_counter_examples() {
        let episodes: Vec<QuestEpisode> = (0..7).map(|_| make_episode()).collect();
        let list = HandoffKind::List { previous_list_at: 4.0 };
        let mut tally = HandoffTally::default();
        tally.add_handoff(&episodes[0], &observation(list, vec![]));
        tally.add_handoff(&episodes[1], &observation(HandoffKind::Pop, vec![Confounder::AlsoCompleted(5)]));
        tally.add_handoff(&episodes[2], &observation(HandoffKind::Pop, vec![]));
        tally.add_handoff(&episodes[3], &observation(list, vec![Confounder::LeftLog(6)]));
        tally.add_counter(&episodes[4], &counter(CounterKind::Alternative));
        tally.add_counter(&episodes[5], &counter(CounterKind::InLog));
        tally.add_counter(&episodes[6], &counter(CounterKind::NotStarted));
        let stats = tally.get(Pair { prerequisite: 1, quest: 2 }).unwrap();
        let texts: Vec<String> = stats.pick_evidence().into_iter().map(|e| e.text).collect();
        assert_eq!(
            texts,
            [
                "turned in 1 at npc 7, its accept dialog for 2 opened 0.4s later",
                "turned in 1 at npc 7, its accept dialog for 2 opened 0.4s later (also: also completed 5)",
                "turned in 1 at npc 7; its list 0.4s later offered 2, its list 6.0s before did not",
                "against: offered 2 while 1 was in the log, not completed",
                "against: offered 2 while 1 was not started",
            ]
        );
        assert_eq!(stats.counts_note(), "pop 1+1?, list 1+1?, against 1 not started / 1 in log, 1 via alternatives");
    }

    #[test]
    fn a_character_counts_once_per_bucket_however_many_sessions_show_it() {
        let first = make_episode();
        let second = QuestEpisode {
            character_key: first.character_key.clone(),
            ..make_episode()
        };
        let other = make_episode();
        let mut tally = HandoffTally::default();
        for episode in [&first, &second, &other] {
            tally.add_handoff(episode, &observation(HandoffKind::Pop, vec![]));
        }
        tally.add_handoff(&second, &observation(HandoffKind::Pop, vec![Confounder::AlsoTurnedIn(9)]));
        let stats = tally.get(Pair { prerequisite: 1, quest: 2 }).unwrap();
        assert_eq!((stats.pop.len(), stats.pop_confounded.len(), stats.support()), (2, 1, 2));
        assert_eq!(stats.pick_evidence().len(), 3, "one example per counted observation");
    }
}
