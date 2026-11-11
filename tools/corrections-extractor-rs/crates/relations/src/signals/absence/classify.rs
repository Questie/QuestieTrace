//! Pass 3 of the absence signal: which Questie field explains how a suspect blocked a quest.
//!
//! For suspect Y of blocked quest X (blocked.rs), the offer moments decide which states of Y
//! are compatible with X being offered:
//!
//! ```text
//!   Y in log blocks | Y completed blocks | field
//!   ----------------+--------------------+-------------------------------------------------------
//!   yes             | yes                | exclusiveTo(X) ∋ Y, or nextQuestInChain(X) = Y when X
//!                   |                    | hands off to Y (plus breadcrumbForQuestId when X is a
//!                   |                    | lead-in from another giver)
//!   yes             | no                 | breadcrumbs(X) ∋ Y when Y hands off to X, else
//!                   |                    | disabledByQuest(X) = Y
//!   no              | yes                | availableUntilCompleted(X) = Y
//! ```
//!
//! "Yes" means no character was offered X in that state of Y; "no" means one was. A state never
//! observed at all defaults to "yes", because exclusiveTo is by far the most common of these in
//! Questie's data. exclusiveTo is one-directional: Y ∋ X needs blocked moments of Y of its own
//! (inferring it from X's blocks alone was right 3 times out of 6).
//!
//! A missing prerequisite P becomes preQuestSingle(X) ∋ P, a missing same-name group one
//! preQuestSingle edge per member, and a prerequisite that another prerequisite of X itself
//! requires is folded into that one (Questie lists only the direct step).

use super::attribute::{attribute, Explanations, Share, SharesByCharacter};
use super::blocked::{BlockKind, BlockedQuest, Explanation};
use super::locality::{LocalReason, Locality};
use super::offers::{Character, OfferIndex, QuestOffers};
use super::params::AbsenceParams;
use super::score::{score_edge, ScoreGroup};
use crate::text::to_fixed;
use crate::eligibility::is_forever_quest_id;
use crate::model::{QuestCatalog, QuestId, RelationCandidate, RelationField};
use indexmap::{IndexMap, IndexSet};
use rustc_hash::{FxBuildHasher, FxHashSet};

struct Edge<'a> {
    quest_id: QuestId,
    field: RelationField,
    target: QuestId,
    /// For preQuestSingle edges, decided after ancestors are folded in (see score_group_of).
    group: ScoreGroup,
    shares: SharesByCharacter<'a>,
    /// Prerequisites only: characters blocked while the prerequisite was not even in their log.
    untaken: Option<FxHashSet<Character<'a>>>,
    contradict: usize,
    note: String,
}

pub fn classify(
    blocked_quests: &IndexMap<QuestId, BlockedQuest, FxBuildHasher>,
    index: &OfferIndex,
    locality: &Locality,
    catalog: &QuestCatalog,
    params: &AbsenceParams,
) -> Vec<RelationCandidate> {
    let mut edges: Vec<Edge> = Vec::new();
    for (&quest_id, blocked) in blocked_quests {
        let offers = &index.quests[&quest_id];
        let explanations = attribute(blocked, params);
        let quest_edges = suspects_of(&explanations, index, catalog)
            .into_iter()
            .flat_map(|target| edges_for(quest_id, target, &explanations, offers, index, params))
            .collect();
        edges.extend(without_ancestors(quest_edges, index));
    }

    let mut candidates: Vec<RelationCandidate> = Vec::new();
    for edge in &edges {
        let unexplained_rate = match blocked_quests.get(&edge.quest_id) {
            Some(blocked) if !blocked.characters.is_empty() => blocked.unexplained.len() as f64 / blocked.characters.len() as f64,
            _ => 0.0,
        };
        let mut shares: Vec<&Share> = edge.shares.values().collect();
        shares.sort_by(|a, b| b.share.total_cmp(&a.share));
        let local = bundle_reason(locality, index, edge.quest_id, edge.target);
        let group = score_group_of(edge);
        let score = score_edge(group, shares.iter().map(|share| share.share), local, is_forever_quest_id(edge.quest_id));
        let score = (score * 1000.0).round() / 1000.0;
        if score < params.min_score {
            continue;
        }
        let likely: Vec<&Share> = shares.iter().copied().filter(|share| share.share >= 0.5).collect();
        let transitions = likely.iter().filter(|share| share.transition).count();
        let note = format!(
            "{}; group {}; local {}; transitions {transitions}; shares {}; unexplained {}%",
            edge.note,
            group.name(),
            local.map_or("none", LocalReason::name),
            format_shares(&shares),
            to_fixed(unexplained_rate * 100.0, 0)
        );
        candidates.push(RelationCandidate {
            quest_id: edge.quest_id,
            field: edge.field,
            target: edge.target,
            score,
            // Characters for whom this was the likely cause of their block; the note lists every share.
            support: likely.len() as u32,
            contradict: edge.contradict as u32,
            evidence: shares.iter().take(5).map(|share| share.evidence.clone()).collect(),
            note: Some(note),
        });
    }
    // By field name, as the TypeScript pipeline sorts (localeCompare agrees with byte order on
    // these ASCII camelCase names).
    candidates.sort_by(|a, b| {
        a.quest_id
            .cmp(&b.quest_id)
            .then_with(|| a.field.name().cmp(b.field.name()))
            .then(a.target.cmp(&b.target))
    });
    candidates
}

/// A prerequisite proven by a character who had never taken it, or one only ever seen in the log.
fn score_group_of(edge: &Edge) -> ScoreGroup {
    match &edge.untaken {
        Some(untaken) if edge.group == ScoreGroup::Prerequisite => {
            let proven = untaken
                .iter()
                .any(|character| edge.shares.get(character).is_some_and(|share| share.share >= 0.5));
            if proven {
                ScoreGroup::Prerequisite
            } else {
                ScoreGroup::PendingPrerequisite
            }
        }
        _ => edge.group,
    }
}

fn format_shares(shares: &[&Share]) -> String {
    let shown: Vec<String> = shares.iter().take(12).map(|share| to_fixed(share.share, 2)).collect();
    let more = if shares.len() > shown.len() {
        format!(",...({})", shares.len())
    } else {
        String::new()
    };
    shown.join(",") + &more
}

/// Copies that complete together are interchangeable, so a tie to any of them counts for all.
fn bundle_reason(locality: &Locality, index: &OfferIndex, quest_id: QuestId, target: QuestId) -> Option<LocalReason> {
    let reasons: Vec<Option<LocalReason>> = index.bundle_of(&target).iter().map(|&member| locality.reason(quest_id, member)).collect();
    reasons
        .iter()
        .flatten()
        .copied()
        .find(|reason| reason.is_strong())
        .or_else(|| reasons.iter().flatten().copied().next())
}

/// Suspect quests, with completion bundles expanded: every copy gets its own edge, except hidden
/// flag quests (not in QuestieDB) that complete alongside a real one.
fn suspects_of(explanations: &Explanations, index: &OfferIndex, catalog: &QuestCatalog) -> IndexSet<QuestId, FxBuildHasher> {
    let mut targets = IndexSet::default();
    for explanation in explanations.keys() {
        let members = match explanation.kind {
            BlockKind::InLog | BlockKind::MissingAny => std::slice::from_ref(&explanation.quest),
            BlockKind::Completed | BlockKind::Missing | BlockKind::Pending => index.bundle_of(&explanation.quest),
        };
        let known: Vec<QuestId> = members.iter().copied().filter(|&member| catalog.quest(member).is_some()).collect();
        targets.extend(if known.is_empty() { members.to_vec() } else { known });
    }
    targets
}

fn edges_for<'a>(
    quest_id: QuestId,
    target: QuestId,
    explanations: &Explanations<'a>,
    offers: &QuestOffers,
    index: &OfferIndex,
    params: &AbsenceParams,
) -> Vec<Edge<'a>> {
    let get = |kind: BlockKind| {
        let quest = match kind {
            BlockKind::InLog | BlockKind::MissingAny => target,
            BlockKind::Completed | BlockKind::Missing | BlockKind::Pending => index.representative(target),
        };
        explanations.get(&Explanation { kind, quest })
    };
    let mut edges = Vec::new();
    let edge = |field: RelationField, target: QuestId, group: ScoreGroup, shares: SharesByCharacter<'a>, contradict: usize, note: String| Edge {
        quest_id,
        field,
        target,
        group,
        shares,
        untaken: None,
        contradict,
        note,
    };

    let missing = get(BlockKind::Missing);
    let pending = get(BlockKind::Pending);
    if missing.is_some() || pending.is_some() {
        let untaken = missing.into_iter().flat_map(|shares| shares.keys().copied()).collect();
        let shares = best_of(&[missing, pending]);
        edges.push(Edge {
            untaken: Some(untaken),
            ..edge(
                RelationField::PreQuestSingle,
                target,
                ScoreGroup::Prerequisite,
                shares,
                0,
                "missing prerequisite".into(),
            )
        });
    }
    if let Some(missing_any) = get(BlockKind::MissingAny) {
        let members = &offers.required_any[&target];
        let ids: Vec<String> = members.iter().map(QuestId::to_string).collect();
        let note = format!("missing any of {}", ids.join("/"));
        for &member in members {
            edges.push(edge(
                RelationField::PreQuestSingle,
                member,
                ScoreGroup::AnyPrerequisite,
                missing_any.clone(),
                0,
                note.clone(),
            ));
        }
    }

    let in_log = get(BlockKind::InLog);
    let completed = get(BlockKind::Completed);
    if in_log.is_none() && completed.is_none() {
        return edges;
    }
    let log_contradict = offers.in_log_when_offered.get(&target).map_or(0, FxHashSet::len);
    let completed_contradict = offers.completed_when_offered.get(&target).map_or(0, FxHashSet::len);
    let log_blocks = log_contradict <= params.max_contradictions;
    let completed_blocks = completed_contradict <= params.max_contradictions;
    let observed = match (in_log.is_some(), completed.is_some()) {
        (true, true) => "log+completed",
        (true, false) => "log",
        _ => "completed",
    };

    if log_blocks && completed_blocks {
        let shares = best_of(&[in_log, completed]);
        let contradict = log_contradict + completed_contradict;
        let chain = index.hands_off(quest_id, target) && index.quests.get(&target).is_some_and(|next| next.required.contains(&quest_id));
        let lead_in = index.is_lead_in(quest_id, target);
        // X hands off to one zone copy of a bundle: that copy is the next step, its siblings are no
        // separate relation (and nextQuestInChain holds a single quest).
        let hands_off_to_sibling = index
            .bundle_of(&target)
            .iter()
            .any(|&member| member != target && index.hands_off(quest_id, member));
        if chain || lead_in {
            let note = format!("blocked by {observed}; {}", if chain { "chain" } else { "lead-in" });
            edges.push(edge(
                RelationField::NextQuestInChain,
                target,
                ScoreGroup::Chain,
                shares.clone(),
                contradict,
                note.clone(),
            ));
            if lead_in && !chain {
                edges.push(edge(RelationField::BreadcrumbForQuestId, target, ScoreGroup::Chain, shares, contradict, note));
            }
        } else if !hands_off_to_sibling {
            let sibling = if index.listed_together(quest_id, target) { "; listed together" } else { "" };
            let note = format!("blocked by {observed}{sibling}");
            edges.push(edge(RelationField::ExclusiveTo, target, ScoreGroup::ExclusiveTo, shares, contradict, note));
        }
    } else if let Some(in_log) = in_log.filter(|_| log_blocks) {
        let (field, group) = if index.is_lead_in(target, quest_id) {
            (RelationField::Breadcrumbs, ScoreGroup::Breadcrumbs)
        } else {
            (RelationField::DisabledByQuest, ScoreGroup::DisabledByQuest)
        };
        // Share of X's offered characters that had already completed Y: high means Y usually comes
        // first, so "Y in log" is more likely a stand-in for an unfinished prerequisite chain.
        let precedes = completed_contradict as f64 / offers.characters.len().max(1) as f64;
        let note = format!("blocked by log only; precedes {}", to_fixed(precedes, 2));
        edges.push(edge(field, target, group, in_log.clone(), log_contradict, note));
    } else if let Some(completed) = completed.filter(|_| completed_blocks) {
        let note = format!("blocked by completed only; offered while in log by {log_contradict}");
        edges.push(edge(
            RelationField::AvailableUntilCompleted,
            target,
            ScoreGroup::AvailableUntilCompleted,
            completed.clone(),
            completed_contradict,
            note,
        ));
    }
    edges
}

/// A character who has not started a chain is missing every step of it, so the blame for X's block
/// splits between X's direct prerequisite P and P's own prerequisites. Questie lists only P, so a
/// prerequisite suspect that another one requires is dropped and its blame handed over once,
/// split evenly when several kept prerequisites require it.
fn without_ancestors<'a>(mut edges: Vec<Edge<'a>>, index: &OfferIndex) -> Vec<Edge<'a>> {
    let prerequisites: Vec<usize> = (0..edges.len()).filter(|&i| edges[i].group == ScoreGroup::Prerequisite).collect();
    let requires = |edges: &[Edge], edge: usize, ancestor: usize| {
        edge != ancestor
            && index
                .quests
                .get(&edges[edge].target)
                .is_some_and(|offers| offers.required.contains(&edges[ancestor].target))
    };
    let mut dropped = vec![false; edges.len()];
    for &ancestor in &prerequisites {
        dropped[ancestor] = prerequisites.iter().any(|&edge| requires(&edges, edge, ancestor));
    }
    // Two quests that each look required by the other (possible within required_tolerance) both
    // stay. Decided in order: an ancestor kept here counts as kept for the ones after it.
    for &ancestor in &prerequisites {
        if dropped[ancestor] && !prerequisites.iter().any(|&edge| !dropped[edge] && requires(&edges, edge, ancestor)) {
            dropped[ancestor] = false;
        }
    }

    // Ancestors are never heirs, so their shares are still the blame computed before any
    // hand-over: each ancestor's share moves exactly once.
    for &ancestor in prerequisites.iter().filter(|&&ancestor| dropped[ancestor]) {
        let heirs: Vec<usize> = prerequisites
            .iter()
            .copied()
            .filter(|&edge| !dropped[edge] && requires(&edges, edge, ancestor))
            .collect();
        let handed_shares = edges[ancestor].shares.clone();
        let untaken = edges[ancestor].untaken.clone().unwrap_or_default();
        for &heir in &heirs {
            let heir = &mut edges[heir];
            for (&character, share) in &handed_shares {
                let handed = share.share / heirs.len() as f64;
                heir.shares
                    .entry(character)
                    .and_modify(|current| current.share = (current.share + handed).min(1.0))
                    .or_insert_with(|| Share {
                        share: handed.min(1.0),
                        ..share.clone()
                    });
            }
            // Never having taken an earlier step means never having taken the later one either.
            if let Some(heir_untaken) = &mut heir.untaken {
                heir_untaken.extend(untaken.iter().copied());
            }
        }
    }
    edges.into_iter().zip(dropped).filter(|(_, dropped)| !dropped).map(|(edge, _)| edge).collect()
}

/// Per character, the sharper of the shares; characters in the order the maps list them.
fn best_of<'a>(maps: &[Option<&SharesByCharacter<'a>>]) -> SharesByCharacter<'a> {
    let mut result = SharesByCharacter::default();
    for map in maps.iter().flatten() {
        for (&character, share) in map.iter() {
            if result.get(character).is_none_or(|best: &Share| share.share > best.share) {
                result.insert(character, share.clone());
            }
        }
    }
    result
}
