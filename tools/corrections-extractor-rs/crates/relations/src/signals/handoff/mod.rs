//! `signal:handoff`: turn-in -> immediate next offer -> candidates/handoff.json.
//!
//! Pass 1 finds hand-offs in every Forever episode ([`detect`]) and tallies them per character
//! ([`tally`]), then derives edges through same-name copies ([`variants`]). Pass 2 re-reads the
//! episodes for what needs the full list of claimed edges: counter-evidence ([`counter`]), which
//! other prerequisites were done at each hand-off (ALL-of vs ANY-of), and when copies were flagged
//! completed (parallel copies vs chain steps). Candidates and scores come from [`candidates`].
//!
//! Also writes reports/handoff-timing.json (turn-in -> next dialog/list delays, used to pick the
//! windows in [`PARAMS`]) and reports/handoff-pairs.json (every observed pair's counts, for review
//! and calibration).
//!
//! Episodes are examined in parallel, but their results are folded in file order: which examples
//! a pair shows, and which observed edge speaks for a copy, depend on that order.

mod candidates;
mod counter;
mod detect;
mod tally;
mod timing;
mod variants;

use self::candidates::{all_of_members, next_in_chain_candidates, prerequisite_candidates, variant_candidates, Scores, SCORES};
use self::counter::{find_counter_evidence, ClaimedPrerequisites, CounterObservation};
use self::detect::{find_handoffs, EpisodeHandoffs, HandoffObservation};
use self::tally::{HandoffTally, Pair, PairStats};
use self::timing::TimingHistogram;
use self::variants::{quests_in_variant_edges, variant_edges, CompletionMoments, SameNameVariants};
use super::{to_json, Findings, Signal, SignalInput};
use crate::eligibility::{is_forever_interface, is_forever_quest_id};
use crate::model::{QuestEpisode, QuestId, RelationCandidate};
use crate::timeline::EpisodeTimeline;
use indexmap::IndexMap;
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use serde::Serialize;
use std::collections::BTreeSet;

pub use self::detect::HandoffParams;

/// Windows from reports/handoff-timing.json (2026-10): 18.9k of 20.3k accept dialogs at the
/// turn-in giver came 0.1-0.5s after the turn-in; the giver's list before a turn-in is a median 3s
/// (p90 12s) old. The list windows are generous because confounders in between are recorded.
pub const PARAMS: HandoffParams = HandoffParams {
    pop_window: 3.0,
    list_before: 300.0,
    list_after: 300.0,
};

pub struct Handoff;

impl Signal for Handoff {
    fn name(&self) -> &'static str {
        "handoff"
    }

    fn describe(&self) -> &'static str {
        "turn-in -> immediate next offer chains"
    }

    fn run(&self, input: &SignalInput) -> Findings {
        run(input)
    }
}

/// candidates/handoff.json `params`: the windows, the scores and what the counts mean.
#[derive(Serialize)]
struct FileParams {
    #[serde(flatten)]
    windows: HandoffParams,
    scores: Scores,
    episodes: &'static str,
    support: &'static str,
    contradict: &'static str,
}

/// One row of reports/handoff-pairs.json: characters per kind of observation.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PairSummary {
    prerequisite: QuestId,
    quest: QuestId,
    pop: usize,
    pop_confounded: usize,
    list: usize,
    list_confounded: usize,
    not_started: usize,
    in_log: usize,
    alternative: usize,
}

impl PairSummary {
    fn of(stats: &PairStats) -> Self {
        PairSummary {
            prerequisite: stats.pair.prerequisite,
            quest: stats.pair.quest,
            pop: stats.pop.len(),
            pop_confounded: stats.pop_confounded.len(),
            list: stats.list.len(),
            list_confounded: stats.list_confounded.len(),
            not_started: stats.not_started.len(),
            in_log: stats.in_log.len(),
            alternative: stats.alternative.len(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PairsReport<'a> {
    generated_at: &'a str,
    pairs: Vec<PairSummary>,
}

/// Quests other observed prerequisites of X were completed or not at one hand-off to X.
type CompanionStates = (Pair, Vec<(QuestId, bool)>);

fn run(input: &SignalInput) -> Findings {
    let catalog = input.catalog;
    let episodes: Vec<&QuestEpisode> = input
        .episodes
        .iter()
        .filter(|episode| is_forever_interface(episode.interface_version))
        .collect();

    let found: Vec<EpisodeHandoffs> = episodes
        .par_iter()
        .map(|episode| find_handoffs(&EpisodeTimeline::new(episode, true), catalog, &PARAMS))
        .collect();
    let mut tally = HandoffTally::default();
    let mut timing = TimingHistogram::default();
    let mut covered: BTreeSet<QuestId> = BTreeSet::new();
    for (&episode, handoffs) in episodes.iter().zip(&found) {
        covered.extend(&handoffs.compared_quests);
        for turn_in in &handoffs.turn_ins {
            timing.add(turn_in);
            if turn_in.giver.is_some() && (turn_in.timing.next_list.is_some() || turn_in.timing.next_detail.is_some()) {
                covered.insert(turn_in.quest_id);
            }
        }
        for observation in &handoffs.observations {
            tally.add_handoff(episode, observation);
            covered.insert(observation.offered);
        }
    }

    let observed: Vec<Pair> = tally.pairs().map(|stats| stats.pair).collect();
    let variants = SameNameVariants::new(catalog);
    // Variant edges are tracked like observed ones so pass 2 can find counter-evidence for them too.
    let variant_edges = variant_edges(&observed, &variants, catalog);
    for edge in &variant_edges {
        tally.stats_for(edge.pair);
    }
    let observed_claims = claimed_prerequisites(observed.iter().copied());
    let claimed = claimed_prerequisites(tally.pairs().map(|stats| stats.pair));
    let mut moments = CompletionMoments::new(quests_in_variant_edges(&variant_edges));

    let second_pass: Vec<(Vec<CounterObservation>, Vec<CompanionStates>)> = episodes
        .par_iter()
        .zip(&found)
        .map(|(episode, handoffs)| {
            let timeline = EpisodeTimeline::new(episode, true);
            let counters = find_counter_evidence(&timeline, &claimed, &variants, catalog);
            let companions = handoffs
                .observations
                .iter()
                .map(|observation| companion_states(&timeline, observation, &observed_claims, &variants))
                .collect();
            (counters, companions)
        })
        .collect();
    for (&episode, (counters, companions)) in episodes.iter().zip(second_pass) {
        for counter in &counters {
            tally.add_counter(episode, counter);
        }
        for (pair, states) in companions {
            tally.add_companion_states(episode, pair, states);
        }
        moments.add(episode);
    }

    let all_of = all_of_members(&tally);
    let mut candidates = prerequisite_candidates(&tally, &all_of);
    candidates.extend(variant_candidates(&variant_edges, &tally, &moments, &all_of));
    candidates.extend(next_in_chain_candidates(&tally));
    candidates.sort_by_key(RelationCandidate::canonical_order);
    // Copies get candidates without being observed; covering them keeps recall honest.
    covered.extend(candidates.iter().map(|candidate| candidate.quest_id));

    let mut by_field: IndexMap<String, usize> = IndexMap::new();
    for candidate in &candidates {
        let copy = if candidate.support == 0 { " (copy)" } else { "" };
        *by_field.entry(format!("{}{copy}", candidate.field)).or_default() += 1;
    }
    let forever_quests: FxHashSet<QuestId> = candidates
        .iter()
        .map(|candidate| candidate.quest_id)
        .filter(|&id| is_forever_quest_id(id))
        .collect();
    eprintln!(
        "handoff: {} episodes ({} non-Forever skipped), {} observed pairs, {} candidates on {} Forever-new quests, {} covered\n  {by_field:?}",
        episodes.len(),
        input.episodes.len() - episodes.len(),
        observed.len(),
        candidates.len(),
        forever_quests.len(),
        covered.len(),
    );

    let params = FileParams {
        windows: PARAMS,
        scores: SCORES,
        episodes: "Forever interface versions (16xxx) and versionless episodes",
        support: "characters with a pop or a newly listed offer after turning in the prerequisite (0 for same-name copies; see note)",
        contradict:
            "prerequisites: always 0, contradicted pairs are not emitted; nextQuestInChain: characters who saw a different quest pop after the same turn-in",
    };
    let pairs = PairsReport {
        generated_at: input.generated_at,
        // Observed pairs come first in the tally, before those added for copies.
        pairs: tally.pairs().take(observed.len()).map(PairSummary::of).collect(),
    };
    Findings {
        input_count: episodes.len(),
        params: to_json(&params),
        covered_quest_ids: covered.into_iter().collect(),
        candidates,
        reports: vec![
            ("handoff-timing", to_json(&timing.report(input.generated_at, PARAMS))),
            ("handoff-pairs", to_json(&pairs)),
        ],
    }
}

/// For each quest X, the prerequisites the pairs claim for it.
fn claimed_prerequisites(pairs: impl IntoIterator<Item = Pair>) -> ClaimedPrerequisites {
    let mut claimed = ClaimedPrerequisites::default();
    for pair in pairs {
        claimed.entry(pair.quest).or_default().insert(pair.prerequisite);
    }
    claimed
}

/// Which of X's other observed prerequisites were done at this hand-off (ALL-of vs ANY-of).
/// Copies of the turned-in quest are flagged completed with it, so they say nothing.
fn companion_states(
    timeline: &EpisodeTimeline,
    observation: &HandoffObservation,
    observed_claims: &ClaimedPrerequisites,
    variants: &SameNameVariants,
) -> CompanionStates {
    let copies = variants.of(observation.turned_in);
    let states = observed_claims
        .get(&observation.offered)
        .into_iter()
        .flatten()
        .filter(|&&other| other != observation.turned_in && !copies.contains(&other))
        .map(|&other| (other, timeline.is_completed_at(other, observation.turn_in_at)))
        .collect();
    let pair = Pair {
        prerequisite: observation.turned_in,
        quest: observation.offered,
    };
    (pair, states)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CatalogQuest, CompletedChange, CompletedSource, CompletedTimeline, QuestEventKind, RelationField, TimedValue};
    use crate::test_support::{catalog, catalog_quest, event, gossip, make_episode, npc};

    /// A character who turns in `prerequisite` at npc 1 at t = 100 and gets `quest`'s accept dialog
    /// right after.
    fn popped(prerequisite: QuestId, quest: QuestId) -> QuestEpisode {
        QuestEpisode {
            quest_log: vec![TimedValue { t: 0.0, v: vec![prerequisite] }, TimedValue { t: 100.2, v: vec![] }],
            quest_events: vec![
                event(99.0, QuestEventKind::Complete, prerequisite, Some(npc(1))),
                event(100.0, QuestEventKind::TurnedIn, prerequisite, Some(npc(1))),
                event(100.4, QuestEventKind::Detail, quest, Some(npc(1))),
            ],
            ..make_episode()
        }
    }

    fn named(id: QuestId, name: &str) -> CatalogQuest {
        CatalogQuest {
            name: Some(name.into()),
            ..catalog_quest(id)
        }
    }

    fn run_on(episodes: &[QuestEpisode], quests: Vec<CatalogQuest>) -> Findings {
        let catalog = catalog(quests);
        Handoff.run(&SignalInput {
            episodes,
            catalog: &catalog,
            generated_at: "2026-11-11T00:00:00.000Z",
        })
    }

    #[test]
    fn observed_edges_extend_to_parallel_copies_and_contradictions_drop_prerequisites() {
        // 10001 -> 10010 popped for three characters; 10002 is a same-name copy of 10001 that one
        // of them had flagged together with it. 10020 popped after 10001 once, but another
        // character was offered 10020 without having started 10001.
        let mut copy_flagged = popped(10001, 10010);
        copy_flagged.completed = Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: vec![],
            changes: vec![CompletedChange {
                t: 100.3,
                add: Some(vec![10001, 10002]),
                remove: None,
            }],
        });
        let offered_unearned = QuestEpisode {
            offers: vec![gossip(50.0, npc(2), &[10020], &[])],
            ..make_episode()
        };
        let mut old_build = popped(10001, 10010);
        old_build.interface_version = Some(11507);
        let episodes = [
            copy_flagged,
            popped(10001, 10010),
            popped(10001, 10010),
            popped(10001, 10020),
            offered_unearned,
            old_build,
        ];
        let quests = vec![named(10001, "Intro"), named(10002, "Intro"), named(10010, "Next"), named(10020, "Other")];

        let found = run_on(&episodes, quests);
        let edges: Vec<_> = found
            .candidates
            .iter()
            .map(|c| (c.quest_id, c.field, c.target, c.support, c.contradict))
            .collect();
        assert_eq!(
            edges,
            [
                (10001, RelationField::NextQuestInChain, 10010, 3, 1),
                (10010, RelationField::PreQuestSingle, 10001, 3, 0),
                (10010, RelationField::PreQuestSingle, 10002, 0, 0),
            ],
            "10020's pop is contradicted and rivalled: no candidate, but its character still counts against 10010"
        );
        assert_eq!(found.input_count, 5, "the Classic build is skipped");
        assert_eq!(found.covered_quest_ids, [10001, 10010, 10020]);
        assert_eq!(
            found.candidates[2].note.as_deref(),
            Some("variant:prerequisite of 10010 <- 10001; pop 3+0?, list 0+0?, against 0 not started / 0 in log, 0 via alternatives")
        );

        let pairs = &found.reports.iter().find(|(name, _)| *name == "handoff-pairs").unwrap().1["pairs"];
        let rows: Vec<(i64, i64, i64)> = pairs
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["prerequisite"].as_i64().unwrap(),
                    row["quest"].as_i64().unwrap(),
                    row["notStarted"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(rows, [(10001, 10010, 0), (10001, 10020, 1)], "observed pairs only, with their counter-evidence");
    }
}
