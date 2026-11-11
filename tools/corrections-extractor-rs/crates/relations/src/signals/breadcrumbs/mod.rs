//! `signal:breadcrumbs`: optional lead-in quests (breadcrumbs) from episodes, as both Questie
//! fields (breadcrumbForQuestId on the breadcrumb, breadcrumbs on its target).
//!
//! Three passes: [`links`] finds the (breadcrumb, target) pairs worth judging from the catalog and
//! every episode, [`observe`] reads what each episode says about those pairs, and [`infer`] counts
//! that per character and scores it. Writes `candidates/breadcrumbs.json` and
//! `reports/breadcrumbs-pairs.json` (every judged pair with its per-kind character counts, for
//! review and calibration).

pub mod infer;
pub mod links;
pub mod observe;

use self::infer::{judge_tally, BreadcrumbResult, Tally, MIN_COVERAGE_CHARACTERS};
use self::links::{BreadcrumbParams, LinkCollector};
use self::observe::{observe_episode, ObservationKind};
use super::{forever_episodes, to_json, Findings, Signal, SignalInput};
use crate::model::{QuestCatalog, QuestEpisode, QuestId};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;

pub struct Breadcrumbs;

impl Signal for Breadcrumbs {
    fn name(&self) -> &'static str {
        "breadcrumbs"
    }

    fn describe(&self) -> &'static str {
        "optional lead-in quests (breadcrumbs) from episodes"
    }

    fn run(&self, input: &SignalInput) -> Findings {
        let params = BreadcrumbParams::default();
        let episodes = forever_episodes(input.episodes);
        let result = infer(&episodes, input.catalog, &params);
        let catalog = input.catalog;

        eprintln!(
            "breadcrumbs: {} Forever episodes, {} observed linked pairs, {} optional+dependent, {} edges, {} covered quests",
            episodes.len(),
            result.tally.pairs.len(),
            result.judgements.len(),
            result.candidates.len() / 2,
            result.covered_quest_ids.len()
        );

        let mut review: Vec<ReviewedPair> = result
            .judgements
            .iter()
            .map(|judgement| {
                let name = |quest_id: QuestId| catalog.quest(quest_id).and_then(|quest| quest.name.as_deref());
                ReviewedPair {
                    breadcrumb: judgement.pair.breadcrumb,
                    target: judgement.pair.target,
                    breadcrumb_name: name(judgement.pair.breadcrumb),
                    target_name: name(judgement.pair.target),
                    score: judgement.score,
                    support: judgement.support,
                    contradict: judgement.contradict,
                    counts: &judgement.counts,
                }
            })
            .collect();
        review.sort_by(|a, b| b.score.total_cmp(&a.score));
        let report = json!({ "linkedPairs": result.tally.pairs.len(), "judged": review.len(), "pairs": review });

        Findings {
            input_count: episodes.len(),
            params: to_json(&FileParams {
                tunables: &params,
                min_coverage_characters: MIN_COVERAGE_CHARACTERS,
            }),
            covered_quest_ids: result.covered_quest_ids,
            candidates: result.candidates,
            reports: vec![("breadcrumbs-pairs", report)],
        }
    }
}

/// candidates/breadcrumbs.json `params`: the tunables and the coverage threshold.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileParams<'a> {
    #[serde(flatten)]
    tunables: &'a BreadcrumbParams,
    min_coverage_characters: usize,
}

/// A judged pair in `reports/breadcrumbs-pairs.json`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewedPair<'a> {
    breadcrumb: QuestId,
    target: QuestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    breadcrumb_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_name: Option<&'a str>,
    score: f64,
    support: u32,
    contradict: u32,
    counts: &'a BTreeMap<ObservationKind, u32>,
}

/// The whole signal over `episodes`: links from all of them first, then each episode's
/// observations, tallied in episode order (the order pairs are first seen in decides ties).
pub fn infer<'e>(episodes: &[&'e QuestEpisode], catalog: &QuestCatalog, params: &BreadcrumbParams) -> BreadcrumbResult<'e> {
    let mut collector = LinkCollector::new(catalog, params);
    for episode in episodes {
        collector.add(episode);
    }
    let links = collector.finish();
    let observations: Vec<_> = episodes.par_iter().map(|episode| observe_episode(episode, &links, catalog, params)).collect();
    let mut tally = Tally::default();
    for (episode, observations) in episodes.iter().zip(observations) {
        tally.add(&episode.character_key, observations);
    }
    judge_tally(tally, catalog)
}

#[cfg(test)]
mod tests {
    use super::infer::BreadcrumbResult;
    use super::links::{BreadcrumbParams, Pair};
    use super::observe::ObservationKind;
    use crate::model::*;
    use crate::test_support::{catalog, catalog_quest, event, gossip, make_episode, npc};

    // B is given by npc 1 and turned in at npc 2, who starts T. A is a second lead-in into T.
    const B: QuestId = 100;
    const T: QuestId = 200;
    const A: QuestId = 300;
    const OTHER: QuestId = 400;

    fn quest(id: QuestId, starter: i64, finisher: i64) -> CatalogQuest {
        CatalogQuest {
            starters: Starters {
                npcs: vec![starter],
                ..Starters::default()
            },
            finishers: Finishers {
                npcs: vec![finisher],
                ..Finishers::default()
            },
            ..catalog_quest(id)
        }
    }

    fn base_quests() -> Vec<CatalogQuest> {
        vec![quest(B, 1, 2), quest(T, 2, 3)]
    }

    fn quest_log(entries: &[(f64, &[QuestId])]) -> Vec<TimedValue<Vec<QuestId>>> {
        entries.iter().map(|&(t, ids)| TimedValue { t, v: ids.to_vec() }).collect()
    }

    fn completed(initial: &[QuestId], added: &[(f64, QuestId)]) -> Option<CompletedTimeline> {
        Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: initial.to_vec(),
            changes: added
                .iter()
                .map(|&(t, id)| CompletedChange {
                    t,
                    add: Some(vec![id]),
                    remove: None,
                })
                .collect(),
        })
    }

    /// Turns in B at npc 2; the server pops T right away and the character accepts it.
    fn does_breadcrumb() -> QuestEpisode {
        QuestEpisode {
            quest_log: quest_log(&[(0.0, &[B]), (101.0, &[]), (103.0, &[T])]),
            completed: completed(&[], &[(101.0, B)]),
            quest_events: vec![
                event(100.0, QuestEventKind::TurnedIn, B, Some(npc(2))),
                event(100.5, QuestEventKind::Detail, T, Some(npc(2))),
                event(102.0, QuestEventKind::Accepted, T, Some(npc(2))),
            ],
            ..make_episode()
        }
    }

    /// Walks up to npc 2 without ever doing B and takes T from its list.
    fn skips_breadcrumb() -> QuestEpisode {
        QuestEpisode {
            quest_log: quest_log(&[(0.0, &[]), (52.0, &[T])]),
            offers: vec![gossip(50.0, npc(2), &[T], &[])],
            quest_events: vec![event(51.0, QuestEventKind::Accepted, T, Some(npc(2)))],
            ..make_episode()
        }
    }

    fn run(episodes: &[QuestEpisode], quests: Vec<CatalogQuest>) -> BreadcrumbResult<'_> {
        let episodes: Vec<&QuestEpisode> = episodes.iter().collect();
        super::infer(&episodes, &catalog(quests), &BreadcrumbParams::default())
    }

    fn edges(result: &BreadcrumbResult) -> Vec<String> {
        let mut edges: Vec<String> = result.candidates.iter().map(|c| format!("{}.{}={}", c.quest_id, c.field, c.target)).collect();
        edges.sort();
        edges
    }

    fn breadcrumb_edge() -> String {
        format!("{B}.breadcrumbForQuestId={T}")
    }

    #[test]
    fn emits_both_questie_fields_for_an_optional_lead_in_whose_turn_in_pops_the_target() {
        let episodes = [does_breadcrumb(), does_breadcrumb(), skips_breadcrumb(), skips_breadcrumb()];
        let result = run(&episodes, base_quests());

        assert_eq!(edges(&result), [breadcrumb_edge(), format!("{T}.breadcrumbs={B}")]);
        // Two free characters (1 - 0.2 * 0.4) times two pops (1 - 0.25²), rounded to 3 decimals.
        assert!((result.candidates[0].score - 0.92 * 0.9375).abs() <= 0.001, "{}", result.candidates[0].score);
        assert_eq!(result.candidates[0].support, 4);
        assert_eq!(result.covered_quest_ids, [B, T]);
    }

    #[test]
    fn does_not_call_a_chain_a_breadcrumb_when_nobody_reached_the_target_without_it() {
        let episodes = [does_breadcrumb(), does_breadcrumb()];
        let result = run(&episodes, base_quests());

        assert!(result.candidates.is_empty());
        assert_eq!(result.covered_quest_ids, [B, T]);
    }

    #[test]
    fn characters_who_only_got_the_target_once_the_breadcrumb_was_done_outweigh_a_stray_free_one() {
        // T missing from npc 2's full list before B, offered once B is turned in: what a chain looks like.
        let blocked_until_b = || QuestEpisode {
            quest_log: quest_log(&[(0.0, &[]), (20.0, &[B]), (101.0, &[])]),
            completed: completed(&[], &[(101.0, B)]),
            offers: vec![gossip(10.0, npc(2), &[], &[]), gossip(110.0, npc(2), &[T], &[])],
            quest_events: vec![
                event(19.0, QuestEventKind::Accepted, B, Some(npc(1))),
                event(100.0, QuestEventKind::TurnedIn, B, Some(npc(2))),
            ],
            ..make_episode()
        };
        let clean_episodes = [does_breadcrumb(), skips_breadcrumb()];
        let chain_like_episodes = [does_breadcrumb(), skips_breadcrumb(), blocked_until_b(), blocked_until_b()];
        let clean = run(&clean_episodes, base_quests());
        let chain_like = run(&chain_like_episodes, base_quests());

        assert!(clean.candidates[0].score > 0.5);
        assert!(chain_like.candidates[0].score < 0.2);
        assert_eq!(chain_like.candidates[0].contradict, 2);
    }

    #[test]
    fn two_characters_offered_the_target_with_the_breadcrumb_still_in_the_log_veto_the_pair() {
        let offered_alongside = || QuestEpisode {
            quest_log: quest_log(&[(0.0, &[B])]),
            offers: vec![gossip(50.0, npc(2), &[T], &[B])],
            ..make_episode()
        };
        let episodes = [does_breadcrumb(), skips_breadcrumb(), offered_alongside(), offered_alongside()];

        assert!(run(&episodes, base_quests()).candidates.is_empty());
    }

    #[test]
    fn a_target_reached_through_another_lead_in_is_not_optional() {
        // T needs A or B: skipping B by doing A says nothing about B being optional.
        let via_a = || QuestEpisode {
            completed: completed(&[A], &[]),
            ..skips_breadcrumb()
        };
        let episodes = [does_breadcrumb(), via_a(), via_a()];
        let quests = [base_quests(), vec![quest(A, 5, 2)]].concat();

        assert!(run(&episodes, quests).candidates.is_empty());
    }

    #[test]
    fn only_characters_who_could_have_taken_the_breadcrumb_count_as_skipping_it() {
        let orc_only_breadcrumb = || {
            vec![
                CatalogQuest {
                    required_races: Some(2),
                    ..quest(B, 1, 2)
                },
                quest(T, 2, 3),
            ]
        };
        let orc = PlayerContext {
            race_id: Some(2),
            class_id: Some(1),
            faction: Some(Faction::Horde),
        };
        let humans = [does_breadcrumb(), skips_breadcrumb()];
        let with_orc = [
            does_breadcrumb(),
            QuestEpisode {
                player: orc,
                ..skips_breadcrumb()
            },
        ];

        assert!(run(&humans, orc_only_breadcrumb()).candidates.is_empty());
        assert!(edges(&run(&with_orc, orc_only_breadcrumb())).contains(&breadcrumb_edge()));
    }

    #[test]
    fn never_links_a_quest_offered_by_the_targets_own_giver() {
        let pops_t = || QuestEpisode {
            completed: completed(&[], &[(101.0, OTHER)]),
            quest_events: vec![
                event(100.0, QuestEventKind::TurnedIn, OTHER, Some(npc(2))),
                event(100.5, QuestEventKind::Detail, T, Some(npc(2))),
            ],
            ..make_episode()
        };
        let episodes = [pops_t(), pops_t(), skips_breadcrumb()];

        assert!(run(&episodes, vec![quest(T, 2, 3), quest(OTHER, 2, 2)]).candidates.is_empty());
    }

    #[test]
    fn keeps_one_target_per_breadcrumb_preferring_the_one_the_server_popped() {
        // npc 2 also starts OTHER, a follow-up of T. Once both are done, npc 1 no longer offers B,
        // which says the same about OTHER as about T; only T was popped.
        let past_both = QuestEpisode {
            completed: completed(&[T, OTHER], &[]),
            offers: vec![gossip(10.0, npc(1), &[], &[])],
            ..make_episode()
        };
        let skips_to_both = QuestEpisode {
            offers: vec![gossip(50.0, npc(2), &[T, OTHER], &[])],
            ..skips_breadcrumb()
        };
        let episodes = [does_breadcrumb(), skips_to_both, past_both];
        let result = run(&episodes, [base_quests(), vec![quest(OTHER, 2, 2)]].concat());
        let for_b: Vec<&RelationCandidate> = result.candidates.iter().filter(|c| c.field == RelationField::BreadcrumbForQuestId).collect();

        let mut judged: Vec<QuestId> = result.judgements.iter().map(|judgement| judgement.pair.target).collect();
        judged.sort();
        assert_eq!(judged, [T, OTHER]);
        assert_eq!(for_b.iter().map(|c| c.target).collect::<Vec<_>>(), [T]);
        assert!(
            for_b[0].note.as_deref().unwrap().contains(&format!("other targets {OTHER} (")),
            "{:?}",
            for_b[0].note
        );
    }

    #[test]
    fn needs_two_characters_whose_breadcrumb_left_the_log_as_the_target_was_accepted_and_ignores_abandons() {
        // B in the log, T accepted, B gone moments later without a turn-in.
        let leaves_log = |kind| QuestEpisode {
            quest_log: quest_log(&[(0.0, &[B]), (52.0, &[T])]),
            quest_events: vec![event(51.0, QuestEventKind::Accepted, T, Some(npc(2))), event(51.5, kind, B, None)],
            ..make_episode()
        };
        // Someone completed B (so "never did B" means something) and someone skipped it.
        let base = || {
            vec![
                QuestEpisode {
                    completed: completed(&[B], &[]),
                    ..make_episode()
                },
                skips_breadcrumb(),
            ]
        };
        let removed_once = [base(), vec![leaves_log(QuestEventKind::Removed)]].concat();
        let removed_twice = [base(), vec![leaves_log(QuestEventKind::Removed), leaves_log(QuestEventKind::Removed)]].concat();
        let abandoned_twice = [base(), vec![leaves_log(QuestEventKind::Abandoned), leaves_log(QuestEventKind::Abandoned)]].concat();

        assert!(run(&removed_once, base_quests()).candidates.is_empty());
        assert!(edges(&run(&removed_twice, base_quests())).contains(&breadcrumb_edge()));
        let abandoned = run(&abandoned_twice, base_quests());
        assert!(abandoned.candidates.is_empty());
        let kinds: Vec<ObservationKind> = abandoned.tally.pairs[&Pair { breadcrumb: B, target: T }].characters.keys().copied().collect();
        assert_eq!(kinds, [ObservationKind::Free]);
    }

    #[test]
    fn trusts_is_breadcrumb_quest_true_for_optionality_and_false_to_rule_the_pair_out() {
        let flagged = |is_breadcrumb| QuestEpisode {
            quest_info: Some(
                [(
                    B,
                    QuestClientInfo {
                        is_breadcrumb: Some(is_breadcrumb),
                        ..QuestClientInfo::default()
                    },
                )]
                .into(),
            ),
            ..does_breadcrumb()
        };
        let flagged_only = [flagged(true)];
        let denied = [flagged(false), skips_breadcrumb(), skips_breadcrumb()];

        assert!(edges(&run(&flagged_only, base_quests())).contains(&breadcrumb_edge()));
        assert!(run(&denied, base_quests()).candidates.is_empty());
    }
}
