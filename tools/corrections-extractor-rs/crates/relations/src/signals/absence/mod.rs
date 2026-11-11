//! `relations signal:absence`: complete offer lists that lack a quest the giver starts -> what
//! blocked it (exclusiveTo, nextQuestInChain, breadcrumbs, disabledByQuest,
//! availableUntilCompleted, missing prerequisites).
//!
//! The pipeline reads every Forever episode twice:
//!
//! ```text
//! offers.rs    pass 1: when quests WERE available, to whom, in which state  -> OfferIndex
//! blocked.rs   pass 2: when they were not, and the suspects for each moment -> Blocked
//! classify.rs  suspect -> Questie field, blame from attribute.rs, score from score.rs
//! ```
//!
//! states.rs reads one episode's log and completed set at a moment, locality.rs says which
//! suspects are plausible partners of a blocked quest before any evidence.

mod attribute;
mod blocked;
mod classify;
mod locality;
mod offers;
mod params;
mod score;
mod states;

use super::{Signal, SignalInput, SignalOutput};
use crate::eligibility::{is_forever_interface, is_forever_quest_id};
use crate::model::{CandidateFile, QuestCatalog, QuestEpisode, QuestId, RelationCandidate};
use blocked::{Blocked, BlockedCollector, SkipReason};
use indexmap::IndexMap;
use locality::Locality;
use offers::{OfferIndex, OfferIndexBuilder};
use params::AbsenceParams;
use rustc_hash::FxHashMap;
use serde::Serialize;

pub struct Absence;

impl Signal for Absence {
    fn name(&self) -> &'static str {
        "absence"
    }

    fn describe(&self) -> &'static str {
        "complete offer lists without a quest -> blockers/exclusivity"
    }

    fn run(&self, input: &SignalInput) -> SignalOutput {
        let params = AbsenceParams::default();
        let result = analyze(input.episodes, input.catalog, params);
        let summary = Summary::of(&result, input.generated_at);
        let AbsenceResult {
            index,
            candidates,
            covered_quest_ids,
            ..
        } = result;
        SignalOutput {
            candidates: CandidateFile {
                signal: self.name().to_owned(),
                generated_at: input.generated_at.to_owned(),
                input_count: index.episodes,
                params: serde_json::to_value(params).expect("the params serialize to JSON"),
                covered_quest_ids,
                candidates,
            },
            reports: vec![("absence-summary", serde_json::to_value(summary).expect("the summary serializes to JSON"))],
        }
    }
}

struct AbsenceResult<'a> {
    index: OfferIndex<'a>,
    blocked: Blocked<'a>,
    candidates: Vec<RelationCandidate>,
    /// Quests with at least one blocked moment that passed gating, ascending.
    covered_quest_ids: Vec<QuestId>,
}

/// The absence signal end to end, over the episodes of Forever builds.
fn analyze<'a>(episodes: &'a [QuestEpisode], catalog: &'a QuestCatalog, params: AbsenceParams) -> AbsenceResult<'a> {
    let forever = || episodes.iter().filter(|episode| is_forever_interface(episode.interface_version));

    let mut offers = OfferIndexBuilder::new(params);
    for episode in forever() {
        offers.add(episode);
    }
    let names: FxHashMap<QuestId, &str> = catalog.quests.iter().filter_map(|(&id, quest)| Some((id, quest.name.as_deref()?))).collect();
    let index = offers.build(|quest_id| names.get(&quest_id).copied());

    let locality = Locality::new(&index, catalog);
    let mut collector = BlockedCollector::new(&index, &locality, catalog, params);
    for episode in forever() {
        collector.add(episode);
    }
    let blocked = collector.finish();

    let candidates = classify::classify(&blocked.quests, &index, &locality, catalog, &params);
    let mut covered_quest_ids: Vec<QuestId> = blocked.quests.keys().copied().collect();
    covered_quest_ids.sort_unstable();
    AbsenceResult {
        index,
        blocked,
        candidates,
        covered_quest_ids,
    }
}

/// reports/absence-summary.json
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary<'r> {
    generated_at: &'r str,
    episodes: usize,
    quests_offered: usize,
    givers: usize,
    blocked_moments: usize,
    /// Most frequent reason first.
    skipped_moments: IndexMap<&'static str, usize>,
    covered_quests: usize,
    covered_forever_quests: usize,
    candidates: usize,
    by_field: IndexMap<&'static str, usize>,
    /// Score >= 0.5 is where measured precision is about 0.75 or better (see score.rs).
    likely: Likely,
    by_score_band: IndexMap<String, usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Likely {
    all: IndexMap<&'static str, usize>,
    forever_quests: IndexMap<&'static str, usize>,
}

impl<'r> Summary<'r> {
    fn of(result: &AbsenceResult, generated_at: &'r str) -> Self {
        let candidates = &result.candidates;
        let mut skipped: Vec<(SkipReason, usize)> = result.blocked.skipped.iter().map(|(&reason, &count)| (reason, count)).collect();
        skipped.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
        let likely: Vec<&RelationCandidate> = candidates.iter().filter(|candidate| candidate.score >= 0.5).collect();
        let field = |candidate: &RelationCandidate| candidate.field.name();
        Summary {
            generated_at,
            episodes: result.index.episodes,
            quests_offered: result.index.quests.len(),
            givers: result.index.quests_by_giver.len(),
            blocked_moments: result.blocked.blocked_moments,
            skipped_moments: skipped.into_iter().map(|(reason, count)| (reason.name(), count)).collect(),
            covered_quests: result.covered_quest_ids.len(),
            covered_forever_quests: result.covered_quest_ids.iter().filter(|&&quest_id| is_forever_quest_id(quest_id)).count(),
            candidates: candidates.len(),
            by_field: count_by(candidates.iter(), field),
            likely: Likely {
                all: count_by(likely.iter().copied(), field),
                forever_quests: count_by(likely.iter().copied().filter(|candidate| is_forever_quest_id(candidate.quest_id)), field),
            },
            by_score_band: count_by(candidates.iter(), |candidate| {
                format!("{:.1}", ((candidate.score * 10.0).floor() / 10.0).min(0.9))
            }),
        }
    }
}

/// Candidates per key, keys in the order first met.
fn count_by<'c, K: std::hash::Hash + Eq>(
    candidates: impl Iterator<Item = &'c RelationCandidate>,
    key_of: impl Fn(&RelationCandidate) -> K,
) -> IndexMap<K, usize> {
    let mut counts = IndexMap::new();
    for candidate in candidates {
        *counts.entry(key_of(candidate)).or_default() += 1;
    }
    counts
}

/// JS `x.toFixed(digits)` for finite numbers below 1e21. Evidence and notes print times, shares
/// and rates this way, and those strings are compared with the TypeScript output: Rust rounds an
/// exact tie to even ("0.12" for 0.125), JS away from zero ("0.13"). With `digits` decimals the
/// exact ties are the odd multiples of 2^-(digits + 1). JS prints -0 as "0".
fn to_fixed(x: f64, digits: usize) -> String {
    let x = x + 0.0;
    let scaled = x.abs() * f64::from(1u32 << (digits + 1));
    let tie = scaled.fract() == 0.0 && scaled % 2.0 == 1.0;
    let away_from_zero = if x > 0.0 { x.next_up() } else { x.next_down() };
    format!("{:.digits$}", if tie { away_from_zero } else { x })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CatalogQuest, CompletedChange, CompletedSource, CompletedTimeline, Faction, Finishers, GiverRef, OfferSnapshot, OfferSource, OfferedQuest,
        PlayerContext, QuestEventKind, RelationField, Starters, TimedValue,
    };
    use crate::test_support::{catalog, catalog_quest, event, gossip, make_episode, npc};
    use QuestEventKind::{Accepted, Detail, TurnedIn};
    use RelationField::{BreadcrumbForQuestId, Breadcrumbs, ExclusiveTo, NextQuestInChain, PreQuestSingle};

    const GIVER: GiverRef = GiverRef {
        kind: crate::model::GiverKind::Npc,
        id: 100,
    };
    // Two variants of one quest (zone copies share a name).
    const X: QuestId = 1001;
    const Y: QuestId = 1002;

    fn named(id: QuestId, name: &str) -> CatalogQuest {
        CatalogQuest {
            name: Some(name.into()),
            ..catalog_quest(id)
        }
    }

    /// Skill-gated like Forever's Camping 101.
    fn variants() -> Vec<CatalogQuest> {
        [X, Y]
            .map(|id| CatalogQuest {
                required_skill: Some((129, 1)),
                ..named(id, "Path of Ash")
            })
            .to_vec()
    }

    fn plain_variants() -> Vec<CatalogQuest> {
        vec![named(X, "Path of Ash"), named(Y, "Path of Ash")]
    }

    fn completed(initial: &[QuestId], changes: &[(f64, &[QuestId])]) -> Option<CompletedTimeline> {
        Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: initial.to_vec(),
            changes: changes
                .iter()
                .map(|&(t, add)| CompletedChange {
                    t,
                    add: Some(add.to_vec()),
                    remove: None,
                })
                .collect(),
        })
    }

    fn quest_log(entries: &[(f64, &[QuestId])]) -> Vec<TimedValue<Vec<QuestId>>> {
        entries.iter().map(|&(t, v)| TimedValue { t, v: v.to_vec() }).collect()
    }

    fn run<'a>(episodes: &'a [QuestEpisode], catalog: &'a QuestCatalog) -> AbsenceResult<'a> {
        analyze(episodes, catalog, AbsenceParams::default())
    }

    fn edge<'r>(result: &'r AbsenceResult, quest_id: QuestId, field: RelationField, target: QuestId) -> Option<&'r RelationCandidate> {
        result
            .candidates
            .iter()
            .find(|candidate| candidate.quest_id == quest_id && candidate.field == field && candidate.target == target)
    }

    fn note(candidate: &RelationCandidate) -> &str {
        candidate.note.as_deref().unwrap_or_default()
    }

    fn skipped(result: &AbsenceResult, reason: SkipReason) -> usize {
        result.blocked.skipped.get(&reason).copied().unwrap_or(0)
    }

    #[test]
    fn reads_exclusive_to_from_a_list_that_loses_x_right_after_y_is_accepted_in_that_direction_only() {
        let chooser = QuestEpisode {
            completed: completed(&[500], &[]),
            quest_log: quest_log(&[(0.0, &[]), (13.0, &[Y])]),
            quest_events: vec![event(11.0, Detail, Y, Some(GIVER)), event(12.0, Accepted, Y, Some(GIVER))],
            offers: vec![gossip(10.0, GIVER, &[X, Y], &[]), gossip(20.0, GIVER, &[], &[])],
            ..make_episode()
        };
        let episodes = [chooser];
        let catalog = catalog(variants());
        let result = run(&episodes, &catalog);

        let forward = edge(&result, X, ExclusiveTo, Y).expect("X exclusiveTo Y");
        assert_eq!(forward.support, 1);
        assert!(forward.score > 0.5);
        assert_eq!(
            forward.evidence[0].text,
            "npc:100 list at 20.0s lacked 1001 (offered at 10.0s); 1002 in log (accepted at 12.0s)"
        );
        // Nothing was observed about Y's availability, so nothing is claimed about what hides Y.
        assert!(edge(&result, Y, ExclusiveTo, X).is_none());
    }

    #[test]
    fn drops_a_blocker_once_another_character_was_offered_x_while_in_that_state() {
        let chooser = QuestEpisode {
            quest_log: quest_log(&[(0.0, &[]), (13.0, &[Y])]),
            quest_events: vec![event(12.0, Accepted, Y, Some(GIVER))],
            offers: vec![gossip(10.0, GIVER, &[X, Y], &[]), gossip(20.0, GIVER, &[], &[])],
            ..make_episode()
        };
        let holds_both = QuestEpisode {
            quest_log: quest_log(&[(0.0, &[Y])]),
            offers: vec![gossip(5.0, GIVER, &[X], &[])],
            ..make_episode()
        };
        let episodes = [chooser, holds_both];
        let catalog = catalog(variants());
        let result = run(&episodes, &catalog);

        // The chooser's block of X is still seen, but nothing consistent explains it any more.
        assert_eq!(result.blocked.blocked_moments, 1);
        assert_eq!(result.blocked.quests[&X].unexplained.len(), 1);
        assert!(result.candidates.iter().all(|candidate| !(candidate.quest_id == X && candidate.target == Y)));
    }

    mod prerequisites {
        //! X stays off the list until P is turned in.
        use super::*;

        const P: QuestId = 2000;
        const NEXT: QuestId = 2001;

        fn quests() -> QuestCatalog {
            catalog([named(P, "First Step"), named(NEXT, "Second Step")])
        }

        fn veteran() -> QuestEpisode {
            QuestEpisode {
                completed: completed(&[P], &[]),
                offers: vec![gossip(5.0, GIVER, &[NEXT], &[])],
                ..make_episode()
            }
        }

        fn learner(log_at_visit: &[QuestId]) -> QuestEpisode {
            QuestEpisode {
                completed: completed(&[], &[(31.0, &[P])]),
                quest_log: quest_log(&[(0.0, log_at_visit), (21.0, &[P]), (31.0, &[])]),
                quest_events: vec![event(20.0, Accepted, P, Some(npc(7))), event(30.0, TurnedIn, P, Some(GIVER))],
                offers: vec![gossip(10.0, GIVER, &[], log_at_visit), gossip(32.0, GIVER, &[NEXT], &[])],
                ..make_episode()
            }
        }

        #[test]
        fn is_proven_by_a_block_while_p_was_not_even_taken() {
            let episodes = [learner(&[]), veteran()];
            let catalog = quests();
            let result = run(&episodes, &catalog);

            let prerequisite = edge(&result, NEXT, PreQuestSingle, P).expect("NEXT requires P");
            assert!(note(prerequisite).contains("group prerequisite;"));
            assert!(prerequisite.score > 0.9);
            assert!(prerequisite.evidence[0].text.ends_with("; 2000 not taken"));
        }

        #[test]
        fn scores_lower_when_p_was_in_the_log_at_every_block_as_a_breadcrumb_looks_the_same() {
            let episodes = [learner(&[P]), veteran()];
            let catalog = quests();
            let result = run(&episodes, &catalog);

            let prerequisite = edge(&result, NEXT, PreQuestSingle, P).expect("NEXT requires P");
            assert!(note(prerequisite).contains("group pendingPrerequisite;"));
            assert!(prerequisite.score < 0.9);
            // P in the log is the same fact as P not completed yet: no separate log-based blocker.
            assert!(result.candidates.iter().all(|candidate| candidate.field == PreQuestSingle));
        }
    }

    fn offered_with_completed(quests: &[QuestId], offered: &[QuestId]) -> QuestEpisode {
        QuestEpisode {
            completed: completed(quests, &[]),
            offers: vec![gossip(5.0, GIVER, offered, &[])],
            ..make_episode()
        }
    }

    #[test]
    fn names_the_direct_prerequisite_not_the_earlier_steps_of_its_chain() {
        let (first, second, third) = (4000, 4001, 4002);
        let episodes = [
            offered_with_completed(&[first, second], &[third]),
            offered_with_completed(&[first, second], &[third]),
            offered_with_completed(&[first], &[second]),
            offered_with_completed(&[], &[first]),
        ];
        let catalog = catalog([first, second, third].map(catalog_quest));
        let result = run(&episodes, &catalog);

        // The newcomer's block splits between "first" and "second" missing; all of it lands on "second"
        // (the other share is the character who had done "first" only).
        let direct = edge(&result, third, PreQuestSingle, second).expect("third requires second");
        assert!(note(direct).contains("; shares 1.00,1.00;"));
        assert!(direct.evidence[0].text.ends_with("; 4001 not taken (1 other suspects)"));
        assert!(edge(&result, third, PreQuestSingle, first).is_none());
    }

    #[test]
    fn hands_an_earlier_steps_blame_over_once_split_between_the_prerequisites_that_require_it() {
        let (root, left, right, goal) = (4100, 4101, 4102, 4103);
        // The catalog starts every quest at GIVER, so all of them are tied and weigh the same as
        // suspects. The sides are offered at another giver: only the newcomer is blocked on goal.
        let side_giver = npc(101);
        let at_giver = |id| CatalogQuest {
            starters: Starters {
                npcs: vec![GIVER.id],
                ..Starters::default()
            },
            ..catalog_quest(id)
        };
        let episodes = [
            offered_with_completed(&[root, left, right], &[goal]),
            offered_with_completed(&[root, left, right], &[goal]),
            QuestEpisode {
                completed: completed(&[root], &[]),
                offers: vec![gossip(5.0, side_giver, &[left, right], &[])],
                ..make_episode()
            },
            offered_with_completed(&[], &[root]),
        ];
        let catalog = catalog([root, left, right, goal].map(at_giver));
        let result = run(&episodes, &catalog);

        // Thirds of the newcomer's block: root's third goes half to each side (1/3 + 1/6), never all
        // of it twice (2/3), and not lost (1/3).
        for side in [left, right] {
            assert!(note(edge(&result, goal, PreQuestSingle, side).expect("goal requires both sides")).contains("; shares 0.50;"));
        }
        assert!(edge(&result, goal, PreQuestSingle, root).is_none());
    }

    #[test]
    fn treats_zone_copies_that_always_complete_together_as_one_suspect_and_names_every_copy() {
        let (copy_a, copy_b, next) = (6001, 6002, 6003);
        let finishers = Finishers {
            npcs: vec![GIVER.id],
            objects: vec![],
        };
        let quests = [
            CatalogQuest {
                finishers: finishers.clone(),
                ..named(copy_a, "Welcome")
            },
            CatalogQuest {
                finishers,
                ..named(copy_b, "Welcome")
            },
            named(next, "Onwards"),
        ];
        // Turning in either copy completes both, in one completed-set batch.
        let veteran = || QuestEpisode {
            completed: completed(&[], &[(3.0, &[copy_a, copy_b])]),
            offers: vec![gossip(5.0, GIVER, &[next], &[])],
            ..make_episode()
        };
        let newcomer = QuestEpisode {
            offers: vec![gossip(5.0, GIVER, &[], &[])],
            ..make_episode()
        };
        let episodes = [veteran(), veteran(), newcomer];
        let catalog = catalog(quests);
        let result = run(&episodes, &catalog);

        for copy in [copy_a, copy_b] {
            let prerequisite = edge(&result, next, PreQuestSingle, copy).expect("next requires each copy");
            assert!(prerequisite.evidence[0].text.ends_with("; 6001/6002 not taken"));
            assert_eq!(prerequisite.support, 1);
        }
    }

    #[test]
    fn reads_any_one_of_these_copies_as_prerequisites_when_no_single_copy_is_required() {
        let (copy_a, copy_b, next) = (5001, 5002, 5003);
        // Both copies are turned in where the next quest starts.
        let finishers = Finishers {
            npcs: vec![GIVER.id],
            objects: vec![],
        };
        let quests = [
            CatalogQuest {
                finishers: finishers.clone(),
                ..named(copy_a, "Welcome")
            },
            CatalogQuest {
                finishers,
                ..named(copy_b, "Welcome")
            },
            named(next, "Onwards"),
        ];
        let episodes = [
            offered_with_completed(&[copy_a], &[next]),
            offered_with_completed(&[copy_b], &[next]),
            QuestEpisode {
                quest_log: quest_log(&[(0.0, &[copy_a])]),
                offers: vec![gossip(5.0, GIVER, &[], &[copy_a])],
                ..make_episode()
            },
        ];
        let catalog = catalog(quests);
        let result = run(&episodes, &catalog);

        for copy in [copy_a, copy_b] {
            let prerequisite = edge(&result, next, PreQuestSingle, copy).expect("next requires either copy");
            assert!(note(prerequisite).starts_with("missing any of 5001/5002;"));
            assert!(prerequisite.evidence[0].text.ends_with("; none of 5001/5002 completed"));
            assert_eq!(prerequisite.support, 1);
        }
        assert!(result.candidates.iter().all(|candidate| candidate.field == PreQuestSingle));
    }

    #[test]
    fn reads_breadcrumbs_when_only_the_lead_in_in_the_log_hides_the_target() {
        let (lead, target) = (3000, 3001);
        let target_giver = npc(302);
        let follower = QuestEpisode {
            completed: completed(&[], &[(21.0, &[lead])]),
            quest_log: quest_log(&[(0.0, &[lead]), (21.0, &[])]),
            quest_events: vec![event(20.0, TurnedIn, lead, Some(target_giver))],
            offers: vec![gossip(10.0, target_giver, &[], &[lead]), gossip(22.0, target_giver, &[target], &[])],
            ..make_episode()
        };
        let skipper = QuestEpisode {
            offers: vec![gossip(5.0, target_giver, &[target], &[])],
            ..make_episode()
        };
        let episodes = [follower, skipper];
        let catalog = catalog([named(lead, "Report to the Outpost"), named(target, "Outpost Trouble")]);
        let result = run(&episodes, &catalog);

        assert!(edge(&result, target, Breadcrumbs, lead).expect("target's breadcrumb").score > 0.4);
    }

    #[test]
    fn reads_a_lead_in_to_another_giver_as_the_next_step_and_breadcrumb_of_its_target() {
        // The lead-in is turned in at the target's giver, which offers the target right after but
        // never the lead-in itself. A character who took the target directly no longer sees the
        // lead-in at its own giver.
        let (lead_in, target) = (7000, 7001);
        let (lead_in_giver, target_giver) = (npc(1), npc(2));
        let follower = QuestEpisode {
            completed: completed(&[], &[(21.0, &[lead_in])]),
            quest_log: quest_log(&[(0.0, &[]), (7.0, &[lead_in]), (21.0, &[])]),
            quest_events: vec![
                event(6.0, Accepted, lead_in, Some(lead_in_giver)),
                event(20.0, TurnedIn, lead_in, Some(target_giver)),
            ],
            offers: vec![gossip(5.0, lead_in_giver, &[lead_in], &[]), gossip(21.0, target_giver, &[target], &[])],
            ..make_episode()
        };
        let skipper = QuestEpisode {
            quest_log: quest_log(&[(0.0, &[]), (7.0, &[target])]),
            quest_events: vec![event(6.0, Accepted, target, Some(target_giver))],
            offers: vec![gossip(5.0, target_giver, &[target], &[]), gossip(30.0, lead_in_giver, &[], &[])],
            ..make_episode()
        };
        let episodes = [follower, skipper];
        let catalog = catalog([named(lead_in, "Word from the Outpost"), named(target, "Outpost Trouble")]);
        let result = run(&episodes, &catalog);

        for field in [NextQuestInChain, BreadcrumbForQuestId] {
            let candidate = edge(&result, lead_in, field, target).expect("the lead-in hands off to the target");
            assert!(note(candidate).starts_with("blocked by log; lead-in; group chain; local handoff;"));
            assert!(candidate.score > 0.9);
        }
        assert!(edge(&result, lead_in, ExclusiveTo, target).is_none());
    }

    #[test]
    fn counts_repeated_reads_with_nothing_changed_in_between_as_one_blocked_moment() {
        let offered = QuestEpisode {
            offers: vec![gossip(5.0, GIVER, &[X, Y], &[])],
            ..make_episode()
        };
        // The completed-set update at 250 adds nothing, but the set is versioned by its updates, not
        // compared by value (as in the TypeScript): the read at 300 is a new observation.
        let rereads = QuestEpisode {
            completed: completed(&[], &[(250.0, &[])]),
            quest_log: quest_log(&[(0.0, &[Y])]),
            offers: vec![gossip(100.0, GIVER, &[], &[]), gossip(200.0, GIVER, &[], &[]), gossip(300.0, GIVER, &[], &[])],
            ..make_episode()
        };
        let episodes = [offered, rereads];
        let catalog = catalog(plain_variants());
        let result = run(&episodes, &catalog);

        assert_eq!(result.blocked.blocked_moments, 2);
    }

    mod gating {
        //! Lists that say nothing about relationships are set aside. An offered character shows X
        //! is available to Y-less characters; the blocked one opens the giver with Y in the log.
        //! Each case changes one thing about the blocked character.
        use super::*;

        fn offered() -> QuestEpisode {
            QuestEpisode {
                offers: vec![gossip(5.0, GIVER, &[X, Y], &[])],
                ..make_episode()
            }
        }

        fn blocked() -> QuestEpisode {
            QuestEpisode {
                quest_log: quest_log(&[(0.0, &[Y])]),
                offers: vec![gossip(10.0, GIVER, &[], &[])],
                ..make_episode()
            }
        }

        fn run_with(blocked: QuestEpisode, quests: Vec<CatalogQuest>, check: impl FnOnce(&AbsenceResult)) {
            let episodes = [offered(), blocked];
            let catalog = catalog(quests);
            check(&run(&episodes, &catalog));
        }

        #[test]
        fn uses_the_plain_case_as_evidence() {
            run_with(blocked(), plain_variants(), |result| {
                let exclusive = edge(result, X, ExclusiveTo, Y).expect("X exclusiveTo Y");
                assert!(exclusive.score > 0.5);
                assert!(exclusive.evidence[0].text.ends_with("; 1002 in log"));
            });
        }

        #[test]
        fn a_level_below_every_level_x_was_offered_at_even_when_the_catalog_allows_it() {
            let blocked = QuestEpisode {
                levels: vec![TimedValue { t: 0.0, v: 8 }],
                ..blocked()
            };
            let quests = vec![
                CatalogQuest {
                    required_level: Some(5),
                    ..named(X, "Path of Ash")
                },
                named(Y, "Path of Ash"),
            ];
            run_with(blocked, quests, |result| {
                assert!(skipped(result, SkipReason::BelowOfferedLevel) > 0);
                assert!(edge(result, X, ExclusiveTo, Y).is_none());
            });
        }

        #[test]
        fn a_faction_x_was_never_offered_to() {
            let blocked = QuestEpisode {
                player: PlayerContext {
                    race_id: Some(2),
                    class_id: Some(1),
                    faction: Some(Faction::Horde),
                },
                ..blocked()
            };
            run_with(blocked, plain_variants(), |result| {
                assert!(skipped(result, SkipReason::UnofferedFaction) > 0);
                assert!(edge(result, X, ExclusiveTo, Y).is_none());
            });
        }

        #[test]
        fn a_list_read_right_after_xs_own_dialog() {
            // Accepting X puts it in the log; a dialog shown without accepting is only a touch.
            for kind in [Accepted, Detail] {
                let blocked = QuestEpisode {
                    quest_events: vec![event(9.5, kind, X, Some(npc(999)))],
                    ..blocked()
                };
                run_with(blocked, plain_variants(), |result| {
                    // Y's read is set aside for Y in the log in the plain case too; X's is the second.
                    assert_eq!(skipped(result, SkipReason::OwnState), 2, "{kind:?}");
                    assert!(edge(result, X, ExclusiveTo, Y).is_none(), "{kind:?}");
                });
            }
        }

        #[test]
        fn an_empty_gossip_sub_menu_right_after_the_quest_list() {
            let blocked = QuestEpisode {
                offers: vec![gossip(8.0, GIVER, &[Y + 1], &[]), gossip(10.0, GIVER, &[], &[])],
                ..blocked()
            };
            run_with(blocked, plain_variants(), |result| {
                assert_eq!(skipped(result, SkipReason::GossipSubMenu), 1);
                assert_eq!(result.blocked.blocked_moments, 1);
            });
        }

        #[test]
        fn a_standalone_quest_detail_which_may_be_a_follow_up_pushed_after_a_turn_in() {
            let detail = OfferSnapshot {
                t: 10.0,
                source: OfferSource::Detail,
                giver: Some(GIVER),
                available: vec![OfferedQuest {
                    id: Y + 1,
                    trivial: None,
                    frequency: None,
                    repeatable: None,
                }],
                unresolved_titles: None,
                active: None,
                list_complete: true,
            };
            let blocked = QuestEpisode {
                offers: vec![detail],
                ..blocked()
            };
            run_with(blocked, plain_variants(), |result| assert_eq!(result.blocked.blocked_moments, 0));
        }

        /// A skill-gated quest needs an offer to the same character in an earlier session. Session
        /// names are missing for a third of sessions, so order comes from session_order only.
        fn in_session(episode: QuestEpisode, session_order: u32) -> QuestEpisode {
            QuestEpisode {
                character_key: "returning".into(),
                session_order,
                session_name: (session_order == 0).then(|| "2026-10-01_10-00-00".into()),
                ..episode
            }
        }

        #[test]
        fn a_skill_gated_quest_counts_a_block_in_a_later_unnamed_session() {
            let episodes = [in_session(offered(), 0), in_session(blocked(), 1)];
            let catalog = catalog(variants());
            let result = run(&episodes, &catalog);

            let exclusive = edge(&result, X, ExclusiveTo, Y).expect("X exclusiveTo Y");
            assert!(exclusive.score > 0.5);
            assert!(note(exclusive).contains("; transitions 1;"));
        }

        #[test]
        fn a_skill_gated_quest_sets_aside_a_block_in_an_earlier_session_than_the_offer() {
            let episodes = [in_session(offered(), 1), in_session(blocked(), 0)];
            let catalog = catalog(variants());
            let result = run(&episodes, &catalog);

            assert!(skipped(&result, SkipReason::UntrackedRequirements) > 0);
            assert!(edge(&result, X, ExclusiveTo, Y).is_none());
        }

        #[test]
        fn a_skill_gated_quest_the_character_was_never_offered_a_same_requirement_copy_of() {
            run_with(blocked(), variants(), |result| {
                assert!(skipped(result, SkipReason::UntrackedRequirements) > 0);
                assert!(edge(result, X, ExclusiveTo, Y).is_none());
            });
        }
    }

    #[test]
    fn to_fixed_rounds_exact_ties_away_from_zero_like_js() {
        // Expected values printed by node: (x).toFixed(d).
        assert_eq!(to_fixed(0.125, 2), "0.13");
        assert_eq!(to_fixed(0.625, 2), "0.63");
        assert_eq!(to_fixed(12.25, 1), "12.3");
        assert_eq!(to_fixed(12.5, 0), "13");
        assert_eq!(to_fixed(2.5, 0), "3");
        // Not ties: the doubles nearest 1.005 and 886.15 lie just below them, so they round down.
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(886.15, 1), "886.1");
        assert_eq!(to_fixed(86.5533093999984, 1), "86.6");
    }
}
