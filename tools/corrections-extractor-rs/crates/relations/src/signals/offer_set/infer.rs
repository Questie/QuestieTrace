//! From per-quest assessments to prerequisite edges.
//!
//! Alive candidates include every quest characters finished before reaching X (starting zones,
//! earlier hubs), so an edge needs evidence that ties P to X:
//!
//! - primary: P is turned in where X is given (link) AND P was the most recent completion before
//!   characters first saw X (latest) AND, among characters who visited X's giver earlier, most
//!   completed P only after that visit (window).
//! - secondary: no primary exists for X, and P was the latest or an immediate completion with a
//!   compatible window, seen by enough characters, none of whom first saw X right after levelling
//!   to its required level (class-trainer quests unlock by level; what came just before is chance).
//!   Mostly quests whose finisher neither the catalog nor the traces know.
//!
//! Then: drop P1 when another choice P2 implies it (P1 completed at every sighting of P2), and
//! pick the field. One remaining choice is a preQuestSingle (with its faction-split OR members,
//! co-completed copies and race-covering variants). Several independent choices, each the last
//! completion for some characters, are a preQuestGroup. Play order alone cannot tell ALL-of from
//! ANY-of when every character did all of them, so a group is "confirmed" only when, for each
//! member, some character had it done at a complete list from X's giver that still did not offer
//! X while another member was missing (see [`all_of_confirmed`]); unconfirmed groups score lower.

use super::aggregate::{CandidateStats, OrGroup, QuestAssessment};
use super::variants::VariantIndex;
use super::Params;
use crate::eligibility::is_forever_quest_id;
use crate::model::{Evidence, QuestId, RelationCandidate, RelationField};
use crate::text::to_fixed;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeTier {
    Primary,
    Secondary,
}

impl EdgeTier {
    fn name(self) -> &'static str {
        match self {
            EdgeTier::Primary => "primary",
            EdgeTier::Secondary => "secondary",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeShape {
    /// P itself qualified.
    Direct,
    /// A member of a faction-split OR group.
    Or,
    /// A same-name quest the server completed together with P.
    Copy,
    /// Added to cover races no observed copy allows.
    Variant,
}

impl EdgeShape {
    fn name(self) -> &'static str {
        match self {
            EdgeShape::Direct => "direct",
            EdgeShape::Or => "or",
            EdgeShape::Copy => "copy",
            EdgeShape::Variant => "variant",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrerequisiteField {
    Single,
    /// `all_of_confirmed`: every member was seen to be insufficient without another member.
    Group {
        all_of_confirmed: bool,
    },
}

impl PrerequisiteField {
    fn relation_field(self) -> RelationField {
        match self {
            PrerequisiteField::Single => RelationField::PreQuestSingle,
            PrerequisiteField::Group { .. } => RelationField::PreQuestGroup,
        }
    }
}

/// Features of one emitted edge; the score is a calibrated lookup over these.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeFeatures {
    pub field: PrerequisiteField,
    /// The quest is Forever-new content (its truth is hand-authored, not inherited from Classic).
    pub forever: bool,
    pub tier: EdgeTier,
    pub shape: EdgeShape,
    pub characters: u32,
    pub support: u32,
    pub contradict: u32,
    pub link: bool,
    pub pending: u32,
    pub background: u32,
    pub fresh: u32,
    pub immediate: u32,
    pub latest: u32,
    /// Share of characters that first saw X right after levelling to its required level.
    pub level_unlocked_share: f64,
    /// Independent choices left for the quest after pruning (1 = preQuestSingle).
    pub remaining: usize,
}

#[derive(Debug)]
pub struct InferredEdge<'a> {
    pub quest_id: QuestId,
    pub target: QuestId,
    pub features: EdgeFeatures,
    pub evidence: &'a [Evidence],
}

fn window_allows(stats: &CandidateStats, params: &Params) -> bool {
    let window = stats.pending + stats.background;
    window == 0 || f64::from(stats.pending) / f64::from(window) >= params.min_pending_rate
}

fn is_primary(stats: &CandidateStats, params: &Params) -> bool {
    stats.link && stats.latest > 0 && window_allows(stats, params)
}

fn is_secondary(stats: &CandidateStats, params: &Params) -> bool {
    stats.support >= params.min_secondary_support && (stats.latest > 0 || stats.immediate > 0) && window_allows(stats, params)
}

/// For every member, some character had it completed at a complete list from X's giver (at a
/// level that allowed X) that did not offer X while another member was still missing. Under
/// ANY-of that list would have offered X, so each member was shown to be insufficient alone.
fn all_of_confirmed(members: &[QuestId], assessment: &QuestAssessment) -> bool {
    members.iter().all(|&member| {
        assessment
            .blocked_lists
            .iter()
            .any(|missing| !missing.contains(&member) && members.iter().any(|&other| other != member && missing.contains(&other)))
    })
}

/// True when P1 was completed at every sighting of P2 (P2 implies P1) but not the other way round.
fn implied(p1: QuestId, p2: QuestId, assessments: &BTreeMap<QuestId, QuestAssessment>) -> bool {
    let p2_implies_p1 = assessments.get(&p2).is_some_and(|of2| of2.alive_stats(p1).is_some());
    let p1_implies_p2 = assessments.get(&p1).is_some_and(|of1| of1.alive_stats(p2).is_some());
    p2_implies_p1 && !p1_implies_p2
}

/// Same-name candidates completed at exactly the same moments as P for every character: some
/// Forever quests (e.g. "The Great Outdoors") flag all their copies completed on one turn-in, and
/// Questie lists every copy. Steps of a same-name chain complete at different times and stay out.
///
/// Copies are kept even when their audience excludes X's (the Horde copy on an Alliance quest),
/// unlike variants: they are observed completed for these very characters, so they are harmless
/// at runtime, and Questie's authored Forever data lists them (164 of 170 such edges on the
/// authoredForever tier, 2026-10-10). On Classic quests Questie never lists Forever's copies,
/// which the score reflects.
fn co_completed<'a>(stats: &CandidateStats, assessment: &'a QuestAssessment, variants: &VariantIndex) -> Vec<&'a CandidateStats> {
    let Some(name) = variants.name_of(stats.target) else {
        return Vec::new();
    };
    assessment
        .alive
        .iter()
        .filter(|other| {
            other.target != stats.target
                && variants.name_of(other.target) == Some(name)
                && other.support == stats.support
                && other.fresh == stats.fresh
                && other.immediate == stats.immediate
                && other.latest == stats.latest
                && other.pending == stats.pending
        })
        .collect()
}

/// One requirement a prerequisite edge set stands for.
enum Choice<'a> {
    /// P itself, with the copies completed together with it; they stand for the same requirement.
    Direct {
        stats: &'a CandidateStats,
        copies: Vec<&'a CandidateStats>,
    },
    /// Any member of a faction-split group.
    Or(&'a OrGroup),
}

impl<'a> Choice<'a> {
    fn stats(&self) -> &'a CandidateStats {
        match self {
            Choice::Direct { stats, .. } => stats,
            Choice::Or(group) => &group.stats,
        }
    }

    /// The quests this choice stands for: the target, or every member of an OR group.
    fn members(&self) -> &'a [QuestId] {
        match self {
            Choice::Direct { stats, .. } => std::slice::from_ref(&stats.target),
            Choice::Or(group) => &group.members,
        }
    }

    fn copies(&self) -> &[&'a CandidateStats] {
        match self {
            Choice::Direct { copies, .. } => copies,
            Choice::Or(_) => &[],
        }
    }
}

fn choose<'a>(assessment: &'a QuestAssessment, variants: &VariantIndex, params: &Params) -> (EdgeTier, Vec<Choice<'a>>) {
    // Forever can flag another faction's same-name copy completed too; a copy nobody who can take
    // X could do is never X's own prerequisite (it can still ride along as a copy).
    let candidates: Vec<&CandidateStats> = assessment
        .alive
        .iter()
        .filter(|stats| !variants.exclusive_audiences(assessment.quest_id, stats.target))
        .collect();

    let build = |qualifies: fn(&CandidateStats, &Params) -> bool| {
        let mut choices: Vec<Choice> = Vec::new();
        // Linked candidates first, so a co-completed group is represented by the copy given here.
        let mut ordered: Vec<&CandidateStats> = candidates.iter().copied().filter(|stats| qualifies(stats, params)).collect();
        ordered.sort_by_key(|stats| (!stats.link, stats.target));
        for stats in ordered {
            if choices.iter().any(|choice| choice.copies().iter().any(|copy| copy.target == stats.target)) {
                continue;
            }
            choices.push(Choice::Direct {
                stats,
                copies: co_completed(stats, assessment, variants),
            });
        }
        choices.extend(assessment.or_groups.iter().filter(|group| qualifies(&group.stats, params)).map(Choice::Or));
        choices
    };

    let primary = build(is_primary);
    if !primary.is_empty() {
        return (EdgeTier::Primary, primary);
    }
    if assessment.level_unlocked > 0 {
        return (EdgeTier::Secondary, Vec::new());
    }
    (EdgeTier::Secondary, build(is_secondary))
}

/// The prerequisite edges of one quest; none when fewer than `min_characters` saw it.
pub fn infer_quest<'a>(
    assessment: &'a QuestAssessment,
    assessments: &BTreeMap<QuestId, QuestAssessment>,
    variants: &VariantIndex,
    params: &Params,
) -> Vec<InferredEdge<'a>> {
    if assessment.characters < params.min_characters {
        return Vec::new();
    }
    let quest_id = assessment.quest_id;
    let (tier, choices) = choose(assessment, variants, params);

    let implied_by_other = |choice: &Choice, index: usize| {
        choices.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && choice
                    .members()
                    .iter()
                    .all(|&p1| other.members().iter().any(|&p2| p2 != p1 && implied(p1, p2, assessments)))
        })
    };
    let kept: Vec<&Choice> = choices
        .iter()
        .enumerate()
        .filter(|&(index, choice)| !implied_by_other(choice, index))
        .map(|(_, choice)| choice)
        .collect();
    let field = if kept.len() >= 2 {
        let targets: Vec<QuestId> = kept.iter().map(|choice| choice.stats().target).collect();
        PrerequisiteField::Group {
            all_of_confirmed: all_of_confirmed(&targets, assessment),
        }
    } else {
        PrerequisiteField::Single
    };

    let features = |stats: &CandidateStats, shape: EdgeShape| EdgeFeatures {
        field,
        forever: is_forever_quest_id(quest_id),
        tier,
        shape,
        characters: assessment.characters,
        support: stats.support,
        contradict: stats.contradict,
        link: stats.link,
        pending: stats.pending,
        background: stats.background,
        fresh: stats.fresh,
        immediate: stats.immediate,
        latest: stats.latest,
        level_unlocked_share: f64::from(assessment.level_unlocked) / f64::from(assessment.characters),
        remaining: kept.len(),
    };
    let mut edges: Vec<InferredEdge<'a>> = Vec::new();
    // The first edge to a target wins.
    let mut add = |target: QuestId, stats: &CandidateStats, shape: EdgeShape, evidence: &'a [Evidence]| {
        if target != quest_id && edges.iter().all(|edge| edge.target != target) {
            edges.push(InferredEdge {
                quest_id,
                target,
                features: features(stats, shape),
                evidence,
            });
        }
    };

    if let PrerequisiteField::Group { .. } = field {
        // A group entry is one quest; an OR of faction copies inside a group is not expressible.
        for choice in &kept {
            if let Choice::Direct { stats, .. } = choice {
                add(stats.target, stats, EdgeShape::Direct, &stats.evidence);
            }
        }
        return edges;
    }

    for choice in &kept {
        for &member in choice.members() {
            let (stats, shape) = match choice {
                Choice::Direct { stats, .. } => (*stats, EdgeShape::Direct),
                // A member is contradicted overall, so normally not alive: the group's summed
                // stats stand for it.
                Choice::Or(group) => (assessment.alive_stats(member).unwrap_or(&group.stats), EdgeShape::Or),
            };
            add(member, stats, shape, &stats.evidence);
        }
        for copy in choice.copies() {
            add(copy.target, copy, EdgeShape::Copy, &copy.evidence);
        }
        // Races the chosen quests exclude would never see X; their same-name variants are what
        // characters of those races complete instead. A variant nobody in X's audience can take
        // (another race or class entirely) never helps.
        let covering: Vec<QuestId> = choice.members().iter().copied().chain(choice.copies().iter().map(|copy| copy.target)).collect();
        if !variants.leaves_races_uncovered(quest_id, &covering) {
            continue;
        }
        for &member in choice.members() {
            for variant in variants.variants_of(member) {
                if !variants.exclusive_audiences(quest_id, variant) {
                    add(variant, choice.stats(), EdgeShape::Variant, &[]);
                }
            }
        }
    }
    edges
}

pub fn to_candidate(edge: &InferredEdge, score: f64) -> RelationCandidate {
    let f = &edge.features;
    let mut note = format!(
        "{}/{}; {}/{} chars had it at every sighting; link={} latest={} immediate={} fresh={} pending={} background={} levelUnlocked={} remaining={}",
        f.tier.name(),
        f.shape.name(),
        f.support,
        f.characters,
        f.link,
        f.latest,
        f.immediate,
        f.fresh,
        f.pending,
        f.background,
        to_fixed(f.level_unlocked_share, 2),
        f.remaining
    );
    if let PrerequisiteField::Group { all_of_confirmed } = f.field {
        note.push_str(&format!(" allOfConfirmed={all_of_confirmed}"));
    }
    RelationCandidate {
        quest_id: edge.quest_id,
        field: f.field.relation_field(),
        target: edge.target,
        score,
        support: f.support,
        contradict: f.contradict,
        evidence: edge.evidence.to_vec(),
        note: Some(note),
    }
}

#[cfg(test)]
mod tests {
    use super::super::aggregate::{FinisherIndex, QuestFold};
    use super::super::observe::{observe_character, prerequisite_candidates};
    use super::super::PARAMS;
    use super::*;
    use crate::model::*;
    use crate::test_support::{catalog, catalog_quest, event, gossip, make_episode};

    const ALLIANCE: u64 = 4_294_967_373;
    const HORDE: u64 = 8_589_934_770;
    const GIVER: GiverRef = GiverRef { kind: GiverKind::Npc, id: 100 };
    /// The defaults, but secondary edges from 2 characters on, so two-character tests reach them.
    const TEST_PARAMS: Params = Params {
        min_secondary_support: 2,
        ..PARAMS
    };

    /// A quest started and/or finished by one NPC.
    fn quest(id: QuestId, starter: Option<i64>, finisher: Option<i64>) -> CatalogQuest {
        CatalogQuest {
            starters: Starters {
                npcs: starter.into_iter().collect(),
                ..Starters::default()
            },
            finishers: Finishers {
                npcs: finisher.into_iter().collect(),
                objects: vec![],
            },
            ..catalog_quest(id)
        }
    }

    fn horde_warrior() -> PlayerContext {
        PlayerContext {
            race_id: Some(2),
            class_id: Some(1),
            faction: Some(Faction::Horde),
        }
    }

    /// One character: arrives at the giver, then does each quest of `chain` in turn at that giver
    /// (offered, turned in), then is offered `quest`. `initial` is already completed.
    /// `co_completed` lists (chain quest, copy) pairs: copies the server flags completed with the
    /// chain quest (0.3s later, after the offer).
    fn play_chain(initial: &[QuestId], chain: &[QuestId], quest: QuestId, co_completed: &[(QuestId, QuestId)]) -> QuestEpisode {
        let mut episode = make_episode();
        let mut offers = vec![gossip(1.0, GIVER, &[], &[])];
        let mut changes = Vec::new();
        let mut t = 10.0;
        for &step in chain {
            offers.push(gossip(t, GIVER, &[step], &[]));
            episode.quest_events.push(event(t + 50.0, QuestEventKind::TurnedIn, step, Some(GIVER)));
            let copies = co_completed.iter().filter(|&&(of, _)| of == step).map(|&(_, copy)| copy);
            changes.push(CompletedChange {
                t: t + 50.3,
                add: Some(std::iter::once(step).chain(copies).collect()),
                remove: None,
            });
            t += 100.0;
        }
        // Offered 0.1s after the last turn-in, before its completed-quest delta.
        offers.push(gossip(if chain.is_empty() { 5.0 } else { t - 49.9 }, GIVER, &[quest], &[]));
        episode.offers = offers;
        episode.completed = Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: initial.to_vec(),
            changes,
        });
        episode
    }

    /// Every quest's assessment, each episode being its own character. Finishers come from the
    /// catalog only.
    fn assess(catalog: &QuestCatalog, episodes: &[QuestEpisode], params: &Params) -> BTreeMap<QuestId, QuestAssessment> {
        let candidates = prerequisite_candidates(catalog);
        let variants = VariantIndex::new(catalog);
        let mut folds: BTreeMap<QuestId, QuestFold> = BTreeMap::new();
        for episode in episodes {
            for view in observe_character(&[episode], catalog, &candidates, params.completion_tolerance_seconds) {
                let fold = folds.entry(view.quest_id).or_insert_with(|| QuestFold::new(view.quest_id));
                fold.add(&view, catalog, params, &variants);
            }
        }
        let finishers = FinisherIndex::new(catalog, std::iter::empty());
        folds
            .into_iter()
            .map(|(id, fold)| (id, fold.finish(catalog, params, &finishers, &variants)))
            .collect()
    }

    /// The quest's edges as (target, features), in the order they were inferred.
    fn infer(catalog: &QuestCatalog, episodes: &[QuestEpisode], quest_id: QuestId) -> Vec<(QuestId, EdgeFeatures)> {
        let assessments = assess(catalog, episodes, &TEST_PARAMS);
        let variants = VariantIndex::new(catalog);
        infer_quest(&assessments[&quest_id], &assessments, &variants, &TEST_PARAMS)
            .into_iter()
            .map(|edge| (edge.target, edge.features))
            .collect()
    }

    fn targets(edges: &[(QuestId, EdgeFeatures)]) -> Vec<QuestId> {
        let mut targets: Vec<QuestId> = edges.iter().map(|(target, _)| *target).collect();
        targets.sort_unstable();
        targets
    }

    const UNIVERSAL: QuestId = 1;
    const P1: QuestId = 10;
    const P2: QuestId = 11;
    const QUEST: QuestId = 20;

    fn chain_catalog() -> QuestCatalog {
        catalog([
            quest(UNIVERSAL, None, Some(999)),
            quest(P1, Some(100), Some(100)),
            quest(P2, Some(100), Some(100)),
            quest(QUEST, Some(100), None),
        ])
    }

    #[test]
    fn keeps_the_quest_turned_in_at_the_giver_just_before_the_offer_and_ignores_what_everyone_did_before() {
        let catalog = chain_catalog();
        let episodes = [play_chain(&[UNIVERSAL], &[P1], QUEST, &[]), play_chain(&[UNIVERSAL], &[P1], QUEST, &[])];
        let edges = infer(&catalog, &episodes, QUEST);
        assert_eq!(targets(&edges), [P1]);
        let features = &edges[0].1;
        assert_eq!(
            (features.field, features.tier, features.link, features.latest),
            (PrerequisiteField::Single, EdgeTier::Primary, true, 2)
        );
        let assessments = assess(&catalog, &episodes, &TEST_PARAMS);
        let universal = assessments[&QUEST].alive_stats(UNIVERSAL).expect("UNIVERSAL survives");
        assert_eq!((universal.support, universal.background, universal.pending, universal.latest), (2, 2, 0, 0));
    }

    #[test]
    fn with_a_contradiction_allowed_counts_a_candidate_the_first_character_lacked() {
        let catalog = chain_catalog();
        let episodes = [
            play_chain(&[UNIVERSAL], &[], QUEST, &[]),
            play_chain(&[UNIVERSAL], &[P1], QUEST, &[]),
            play_chain(&[UNIVERSAL], &[P1], QUEST, &[]),
        ];
        let one_allowed = Params {
            max_contradictions: 1,
            ..TEST_PARAMS
        };
        let assessments = assess(&catalog, &episodes, &one_allowed);
        let p1 = assessments[&QUEST].alive_stats(P1).expect("P1 lacked by one character only");
        assert_eq!((p1.support, p1.contradict, p1.pending, p1.background, p1.latest), (2, 1, 2, 0, 2));
    }

    #[test]
    fn drops_a_candidate_once_any_character_was_offered_the_quest_without_it() {
        let catalog = chain_catalog();
        let episodes = [
            play_chain(&[UNIVERSAL], &[P1], QUEST, &[]),
            play_chain(&[UNIVERSAL], &[], QUEST, &[]),
            play_chain(&[UNIVERSAL], &[P1], QUEST, &[]),
        ];
        assert_eq!(targets(&infer(&catalog, &episodes, QUEST)), Vec::<QuestId>::new());
    }

    #[test]
    fn lists_only_the_nearest_step_of_a_chain() {
        let catalog = chain_catalog();
        let episodes = [play_chain(&[UNIVERSAL], &[P1, P2], QUEST, &[]), play_chain(&[UNIVERSAL], &[P1, P2], QUEST, &[])];
        assert_eq!(targets(&infer(&catalog, &episodes, QUEST)), [P2]);
    }

    #[test]
    fn drops_a_qualifying_step_that_the_next_qualifying_step_implies() {
        // A talk-to chain: P1 turned in, P2 offered only then and turned in right away, QUEST
        // offered: both are the latest completion (within the tie), linked and pending. P2 is only
        // ever seen with P1 completed, so P1 is implied and P2 alone is the prerequisite.
        let talk_to_chain = || {
            let mut episode = make_episode();
            episode.offers = vec![
                gossip(1.0, GIVER, &[], &[]),
                gossip(10.0, GIVER, &[P1], &[]),
                gossip(60.2, GIVER, &[P2], &[]),
                gossip(61.1, GIVER, &[QUEST], &[]),
            ];
            episode.quest_events = vec![
                event(60.0, QuestEventKind::TurnedIn, P1, Some(GIVER)),
                event(61.0, QuestEventKind::TurnedIn, P2, Some(GIVER)),
            ];
            episode
        };
        let edges = infer(&chain_catalog(), &[talk_to_chain(), talk_to_chain()], QUEST);
        assert_eq!(targets(&edges), [P2]);
        assert_eq!(edges[0].1.field, PrerequisiteField::Single);
    }

    #[test]
    fn makes_a_confirmed_pre_quest_group_when_the_givers_list_lacked_the_quest_with_only_one_member_done() {
        // play_chain shows the giver's list after the first turn-in: it offers the second member, not QUEST.
        let catalog = chain_catalog();
        let episodes = [play_chain(&[UNIVERSAL], &[P1, P2], QUEST, &[]), play_chain(&[UNIVERSAL], &[P2, P1], QUEST, &[])];
        let edges = infer(&catalog, &episodes, QUEST);
        assert_eq!(targets(&edges), [P1, P2]);
        let confirmed = PrerequisiteField::Group { all_of_confirmed: true };
        assert!(edges.iter().all(|(_, features)| features.field == confirmed));
    }

    #[test]
    fn leaves_a_group_unconfirmed_when_play_order_is_the_only_evidence() {
        // Both turned in at the giver with no list read in between, in either order, then QUEST
        // offered: ANY-of fits too.
        let turn_in_both = |order: [QuestId; 2]| {
            let mut episode = make_episode();
            episode.completed = Some(CompletedTimeline {
                source: CompletedSource::Modern,
                initial: vec![UNIVERSAL],
                changes: vec![],
            });
            episode.quest_events = vec![
                event(10.0, QuestEventKind::TurnedIn, order[0], Some(GIVER)),
                event(110.0, QuestEventKind::TurnedIn, order[1], Some(GIVER)),
            ];
            episode.offers = vec![gossip(210.0, GIVER, &[QUEST], &[])];
            episode
        };
        let edges = infer(&chain_catalog(), &[turn_in_both([P1, P2]), turn_in_both([P2, P1])], QUEST);
        assert_eq!(targets(&edges), [P1, P2]);
        let unconfirmed = PrerequisiteField::Group { all_of_confirmed: false };
        assert!(edges.iter().all(|(_, features)| features.field == unconfirmed));
    }

    #[test]
    fn does_not_judge_quests_seen_by_too_few_characters() {
        let edges = infer(&chain_catalog(), &[play_chain(&[UNIVERSAL], &[P1], QUEST, &[])], QUEST);
        assert!(edges.is_empty());
    }

    #[test]
    fn emits_no_secondary_edge_when_characters_first_saw_the_quest_right_after_reaching_its_level() {
        // P1 finishes elsewhere (no link), so only the secondary tier could pick it up.
        let unlinked = catalog([
            quest(P1, None, Some(555)),
            CatalogQuest {
                required_level: Some(11),
                ..quest(QUEST, Some(100), None)
            },
        ]);
        let offered_at_level = |levels: Vec<TimedValue<i64>>| {
            let mut episode = make_episode();
            episode.completed = Some(CompletedTimeline {
                source: CompletedSource::Modern,
                initial: vec![],
                changes: vec![CompletedChange {
                    t: 100.0,
                    add: Some(vec![P1]),
                    remove: None,
                }],
            });
            episode.offers = vec![gossip(160.0, GIVER, &[QUEST], &[])];
            episode.levels = levels;
            episode
        };
        let at_11 = || vec![TimedValue { t: 0.0, v: 11 }];
        let levelled = vec![TimedValue { t: 0.0, v: 10 }, TimedValue { t: 150.0, v: 11 }];
        assert_eq!(targets(&infer(&unlinked, &[offered_at_level(at_11()), offered_at_level(at_11())], QUEST)), [P1]);
        assert!(infer(&unlinked, &[offered_at_level(levelled), offered_at_level(at_11())], QUEST).is_empty());
    }

    const P_ALLIANCE: QuestId = 30;
    const P_HORDE: QuestId = 31;
    const SCOUT_HORDE: QuestId = 32;
    const SHARED: QuestId = 40;
    const SCOUT: QuestId = 50;
    const AFTER_SCOUT: QuestId = 60;
    const OUTDOORS: QuestId = 70;
    const OUTDOORS_HORDE: QuestId = 71;
    const CAMPING: QuestId = 72;

    fn named(id: QuestId, name: &str, required_races: u64, finisher: i64) -> CatalogQuest {
        CatalogQuest {
            name: Some(name.into()),
            required_races: Some(required_races),
            ..quest(id, None, Some(finisher))
        }
    }

    fn variant_catalog() -> QuestCatalog {
        catalog([
            named(P_ALLIANCE, "Report In", ALLIANCE, 100),
            named(P_HORDE, "Report In", HORDE, 100),
            quest(SHARED, Some(100), None),
            named(SCOUT, "Scout Ahead", ALLIANCE, 100),
            named(SCOUT_HORDE, "Scout Ahead", HORDE, 200),
            quest(AFTER_SCOUT, Some(100), None),
            named(OUTDOORS, "The Great Outdoors", ALLIANCE, 100),
            named(OUTDOORS_HORDE, "The Great Outdoors", HORDE, 200),
            CatalogQuest {
                required_races: Some(ALLIANCE),
                ..quest(CAMPING, Some(100), None)
            },
        ])
    }

    #[test]
    fn makes_an_or_of_same_name_variants_when_each_factions_characters_had_their_own_copy() {
        let horde = |mut episode: QuestEpisode| {
            episode.player = horde_warrior();
            episode
        };
        let episodes = [
            play_chain(&[], &[P_ALLIANCE], SHARED, &[]),
            play_chain(&[], &[P_ALLIANCE], SHARED, &[]),
            horde(play_chain(&[], &[P_HORDE], SHARED, &[])),
            horde(play_chain(&[], &[P_HORDE], SHARED, &[])),
        ];
        let edges = infer(&variant_catalog(), &episodes, SHARED);
        assert_eq!(targets(&edges), [P_ALLIANCE, P_HORDE]);
        assert!(edges
            .iter()
            .all(|(_, features)| features.field == PrerequisiteField::Single && features.shape == EdgeShape::Or));
    }

    #[test]
    fn adds_the_other_factions_variant_when_only_one_faction_was_observed() {
        let episodes = [play_chain(&[], &[SCOUT], AFTER_SCOUT, &[]), play_chain(&[], &[SCOUT], AFTER_SCOUT, &[])];
        assert_eq!(targets(&infer(&variant_catalog(), &episodes, AFTER_SCOUT)), [SCOUT_HORDE, SCOUT]);
    }

    #[test]
    fn adds_only_variants_that_the_quests_own_audience_can_take() {
        const NIGHT_ELF: u64 = 8;
        const TAUREN: u64 = 32;
        const SKYBORNE_ALLIANCE: u64 = 4_294_967_296;
        const DRUID: u64 = 1024;
        const WARRIOR: u64 = 1;
        const TRIAL: QuestId = 80;
        let trial = |id: QuestId, required_races: u64, required_classes: u64| CatalogQuest {
            name: Some("Trial".into()),
            required_races: Some(required_races),
            required_classes: Some(required_classes),
            ..quest(id, None, Some(100 + id))
        };
        let audience_catalog = catalog([
            trial(TRIAL, NIGHT_ELF, DRUID),
            trial(81, TAUREN, DRUID),
            trial(82, SKYBORNE_ALLIANCE, DRUID),
            trial(84, SKYBORNE_ALLIANCE, WARRIOR),
            CatalogQuest {
                required_races: Some(NIGHT_ELF + SKYBORNE_ALLIANCE),
                required_classes: Some(DRUID),
                ..quest(83, Some(100), None)
            },
        ]);
        let night_elf_druid = || {
            let mut episode = play_chain(&[], &[TRIAL], 83, &[]);
            episode.player = PlayerContext {
                race_id: Some(4),
                class_id: Some(11),
                faction: Some(Faction::Alliance),
            };
            episode
        };
        // Quest 83 is for Night Elf and Skyborne druids: no one who can take it can take the
        // Tauren copy (race) or the Skyborne warrior copy (class).
        assert_eq!(targets(&infer(&audience_catalog, &[night_elf_druid(), night_elf_druid()], 83)), [TRIAL, 82]);
    }

    #[test]
    fn lists_copies_the_server_completed_together_as_one_pre_quest_single_not_a_group() {
        // The copy lands with the delayed completed-quest delta, just after the next offer.
        let together = [(OUTDOORS, OUTDOORS_HORDE)];
        let episodes = [
            play_chain(&[], &[OUTDOORS], CAMPING, &together),
            play_chain(&[], &[OUTDOORS], CAMPING, &together),
        ];
        let edges = infer(&variant_catalog(), &episodes, CAMPING);
        let shapes: Vec<_> = edges.iter().map(|(target, features)| (*target, features.field, features.shape)).collect();
        assert_eq!(
            shapes,
            [
                (OUTDOORS, PrerequisiteField::Single, EdgeShape::Direct),
                (OUTDOORS_HORDE, PrerequisiteField::Single, EdgeShape::Copy)
            ]
        );
    }
}
