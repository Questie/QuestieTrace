//! The `Assumption:` comment on each field of foreverQuestRelationTraces.lua: why a written
//! relation is believed, in one line a QuestieDB maintainer can read without the review report.
//!
//! Per target it says what the traces saw (the phrase of the first agreeing signal in
//! [`SIGNAL_ORDER`]), which trace signals agree, how certain the combined score is, and how many
//! characters back it, both as ranges. It carries no exact counts or scores: those move with every
//! export, and the module is committed, so they would make every comment change;
//! relations-review.md has the full evidence for every relation. Targets whose reasons differ only by id share one, so a large exclusive group stays one
//! readable line.

use super::edges::{Claim, EdgeKind};
use super::resolve::Outcome;
use super::views::{EdgeView, QuestView};
use crate::model::{QuestId, RelationField};
use indexmap::IndexMap;

/// The order signals are named in, and whose phrase leads the comment. Fixed, so the wording only
/// changes when a signal starts or stops agreeing, not when evidence shifts which one weighs most.
const SIGNAL_ORDER: [&str; 5] = ["handoff", "offer-set", "breadcrumbs", "absence", "cooccurrence"];

/// The combined score as a band; band edges at the acceptance threshold (0.9) and above it.
fn certainty(probability: f64) -> &'static str {
    if probability >= 0.99 {
        "very high"
    } else if probability >= 0.95 {
        "high"
    } else if probability >= 0.9 {
        "medium"
    } else {
        "low"
    }
}

/// The most independent characters any agreeing signal saw it in, as a range. Support only grows
/// as traces accumulate, and above 100 more characters tell a reviewer nothing, so three ranges:
/// between two real exports 4.8% of relations changed range, against 12.2% with a 1000+ range.
fn characters(edge: &EdgeView) -> &'static str {
    match edge.characters() {
        0..=9 => "<10 characters",
        10..=99 => "10+ characters",
        _ => "100+ characters",
    }
}

/// What one signal saw, as said on the field the comment sits on (so both sides of a breadcrumb
/// read naturally). The commented quest is "it"; `other` is the quest (or quests) in the field.
fn phrase(field: RelationField, signal: &str, other: &str) -> Option<String> {
    use RelationField as F;
    let text = match (field, signal) {
        (F::PreQuestSingle | F::PreQuestGroup, "handoff") => format!("offered right after {other} was turned in"),
        (F::PreQuestSingle | F::PreQuestGroup, "offer-set") => format!("{other} was done whenever it was offered"),
        (F::PreQuestSingle | F::PreQuestGroup, "cooccurrence") => format!("everyone who took it had done {other}"),
        (F::PreQuestSingle | F::PreQuestGroup, "absence") => format!("left off complete offer lists until {other} was done"),
        (F::NextQuestInChain, "handoff") => format!("turning it in led straight to {other}"),
        (F::NextQuestInChain, "absence") => format!("left off offer lists once {other} was taken or done"),
        (F::ExclusiveTo, "cooccurrence") => format!("the server completes it and {other} together"),
        (F::ExclusiveTo, "absence") => format!("offer lists show it and {other} exclude each other"),
        (F::BreadcrumbForQuestId, "breadcrumbs") => format!("optional lead-in to {other}"),
        (F::Breadcrumbs, "breadcrumbs") => format!("{other} is an optional lead-in to it"),
        (F::AvailableUntilCompleted, "absence") => format!("left off offer lists once {other} was done"),
        (F::DisabledByQuest, "absence") => format!("left off offer lists while {other} was in the log"),
        _ => return None,
    };
    Some(text)
}

/// Absence sees a breadcrumb B -> T from one side only, and the claim's field says which: B hidden
/// once T is taken (breadcrumbForQuestId) or T hidden while B is in the log (breadcrumbs).
fn is_one_sided_breadcrumb(claim: &Claim, edge: &EdgeView) -> bool {
    edge.kind() == EdgeKind::Breadcrumb && claim.signal() == "absence"
}

fn observation(claim: &Claim, edge: &EdgeView, quest_id: QuestId, other: &str) -> String {
    if is_one_sided_breadcrumb(claim, edge) {
        let quest = quest_id.to_string();
        let (b, t) = if edge.field == RelationField::BreadcrumbForQuestId {
            (quest.as_str(), other)
        } else {
            (other, quest.as_str())
        };
        return if claim.field == RelationField::Breadcrumbs {
            format!("{t} left off offer lists while {b} was in the log")
        } else {
            format!("{b} left off offer lists once {t} was taken or done")
        };
    }
    phrase(edge.field, claim.signal(), other).unwrap_or_else(|| format!("{} signal", claim.signal()))
}

/// The agreeing signals, each once: those in [`SIGNAL_ORDER`] in that order, then any others by name.
fn signals_in_order<'e>(edge: &'e EdgeView) -> Vec<&'e str> {
    let mut signals: Vec<&str> = edge.claims.iter().map(Claim::signal).collect();
    signals.sort_by_key(|signal| (SIGNAL_ORDER.iter().position(|known| known == signal).unwrap_or(SIGNAL_ORDER.len()), *signal));
    signals.dedup();
    signals
}

/// The claim whose phrase leads: the first signal in order that has words for this field.
fn leading_claim<'e>(edge: &'e EdgeView, signals: &[&str]) -> Option<&'e Claim<'e>> {
    let has_words = |claim: &Claim| is_one_sided_breadcrumb(claim, edge) || phrase(edge.field, claim.signal(), "").is_some();
    let in_order = || signals.iter().filter_map(|signal| edge.claims.iter().find(|claim| claim.signal() == *signal));
    in_order().find(|claim| has_words(claim)).or_else(|| in_order().next())
}

fn reason(edge: &EdgeView, quest_id: QuestId, other: &str) -> String {
    let certainty = certainty(edge.probability);
    if edge.outcome == Outcome::Derived {
        return format!("implied by its breadcrumb to {other} ({certainty})");
    }
    let signals = signals_in_order(edge);
    let Some(claim) = leading_claim(edge, &signals) else {
        return certainty.to_owned();
    };
    let mut text = format!(
        "{} ({}; {certainty}; {})",
        observation(claim, edge, quest_id, other),
        signals.join(", "),
        characters(edge)
    );
    if edge.outcome == Outcome::Clique {
        text.push_str(", kept below the min score for its accepted exclusive group");
    }
    if edge.outcome == Outcome::AndGroup {
        text.push_str(", kept below the min score to complete its accepted AND list");
    }
    text
}

/// One field's comment: a reason per target, in the order the field lists them; None without
/// written edges.
pub fn assumption(view: &QuestView, field: RelationField) -> Option<String> {
    // Keyed by the reason with the target left out, so identical reasons merge into one.
    let mut groups: IndexMap<String, (&EdgeView, Vec<QuestId>)> = IndexMap::new();
    for &target in view.relation(field) {
        let Some(edge) = view.edges.iter().find(|edge| edge.is_written() && edge.field == field && edge.target == target) else {
            continue;
        };
        groups
            .entry(reason(edge, view.quest_id, "#"))
            .or_insert_with(|| (edge, Vec::new()))
            .1
            .push(target);
    }
    if groups.is_empty() {
        return None;
    }
    let reasons: Vec<String> = groups
        .values()
        .map(|(edge, targets)| {
            let targets: Vec<String> = targets.iter().map(ToString::to_string).collect();
            reason(edge, view.quest_id, &targets.join(", "))
        })
        .collect();
    Some(format!("Assumption: {}", reasons.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::crossval::Contribution;
    use crate::combine::edges::{Domain, EdgeKey, Reference, Source};
    use crate::model::RelationSet;

    fn claim(signal: &'static str, field: RelationField, score: f64, support: u32) -> Claim<'static> {
        Claim {
            source: Source { signal, field },
            field,
            score,
            support,
            contradict: 0,
            evidence: &[],
        }
    }

    fn edge<'s>(kind: EdgeKind, field: RelationField, target: QuestId, probability: f64, claims: &'s [Claim<'s>]) -> EdgeView<'s> {
        EdgeView {
            key: EdgeKey::new(kind, 0, target),
            field,
            target,
            probability,
            held_out: false,
            outcome: Outcome::Accepted,
            because: None,
            status: None,
            claims,
            contributions: &[],
        }
    }

    fn view<'s>(quest_id: QuestId, relations: &'s RelationSet, edges: Vec<EdgeView<'s>>) -> QuestView<'s> {
        QuestView {
            quest_id,
            name: None,
            domain: Domain::Forever,
            reference: Reference::None,
            relations,
            edges,
            reference_only: vec![],
            note: None,
        }
    }

    use RelationField::{BreadcrumbForQuestId, Breadcrumbs, ExclusiveTo, NextQuestInChain, PreQuestSingle};

    #[test]
    fn leads_with_the_first_signal_in_a_fixed_order_whatever_moved_the_score_most() {
        let claims = [
            claim("absence", PreQuestSingle, 0.94, 1),
            claim("offer-set", PreQuestSingle, 0.98, 13),
            claim("handoff", PreQuestSingle, 0.97, 8),
        ];
        // offer-set moved the score most; the comment still leads with handoff and keeps the order.
        let contributions = [
            Contribution {
                signal: "offer-set",
                log_odds: 2.7,
            },
            Contribution {
                signal: "handoff",
                log_odds: 0.4,
            },
        ];
        let prerequisite = EdgeView {
            contributions: &contributions,
            ..edge(EdgeKind::Prerequisite, PreQuestSingle, 76156, 0.99523, &claims)
        };
        let relations = RelationSet::from([(PreQuestSingle, vec![76156])]);

        assert_eq!(
            assumption(&view(76160, &relations, vec![prerequisite]), PreQuestSingle).as_deref(),
            Some("Assumption: offered right after 76156 was turned in (handoff, offer-set, absence; very high; 10+ characters)")
        );
    }

    #[test]
    fn bands_the_score_with_edges_at_0_99_0_95_and_the_0_9_threshold() {
        let bands: Vec<&str> = [0.99, 0.989, 0.95, 0.949, 0.9, 0.899].into_iter().map(certainty).collect();

        assert_eq!(bands, ["very high", "high", "high", "medium", "medium", "low"]);
    }

    #[test]
    fn ranges_characters_by_the_best_supported_agreeing_signal_at_10_and_100() {
        let ranges: Vec<&str> = [0, 9, 10, 99, 100]
            .into_iter()
            .map(|support| {
                // A weak second signal never lowers the range the best-supported one sets.
                let claims = [claim("absence", PreQuestSingle, 0.9, 1), claim("handoff", PreQuestSingle, 0.9, support)];
                characters(&edge(EdgeKind::Prerequisite, PreQuestSingle, 1, 0.95, &claims))
            })
            .collect();

        assert_eq!(
            ranges,
            ["<10 characters", "<10 characters", "10+ characters", "10+ characters", "100+ characters"]
        );
    }

    #[test]
    fn gives_one_reason_per_target_in_field_order_and_ignores_edges_that_were_not_written() {
        let claims = [claim("cooccurrence", PreQuestSingle, 0.7, 1)];
        let near_miss = EdgeView {
            outcome: Outcome::BelowThreshold,
            ..edge(EdgeKind::Prerequisite, PreQuestSingle, 79009, 0.5, &claims)
        };
        let edges = vec![
            edge(EdgeKind::Prerequisite, PreQuestSingle, 79008, 0.97, &claims),
            edge(EdgeKind::Prerequisite, PreQuestSingle, 79007, 0.92, &claims),
            near_miss,
        ];
        let relations = RelationSet::from([(PreQuestSingle, vec![79007, 79008])]);
        let view = view(79192, &relations, edges);

        assert_eq!(
            assumption(&view, PreQuestSingle).as_deref(),
            Some(
                "Assumption: everyone who took it had done 79007 (cooccurrence; medium; <10 characters); \
                 everyone who took it had done 79008 (cooccurrence; high; <10 characters)"
            )
        );
        assert_eq!(assumption(&view, NextQuestInChain), None);
    }

    #[test]
    fn merges_targets_whose_reasons_differ_only_by_id() {
        // Counts in one range merge; a different band or range splits.
        let busy = [claim("cooccurrence", ExclusiveTo, 0.9, 171)];
        let busier = [claim("cooccurrence", ExclusiveTo, 0.9, 4210)];
        let unpinned = [claim("cooccurrence", ExclusiveTo, 0.9, 0)];
        let edges = vec![
            edge(EdgeKind::Exclusive, ExclusiveTo, 96102, 0.993, &busy),
            edge(EdgeKind::Exclusive, ExclusiveTo, 97925, 0.991, &busier),
            edge(EdgeKind::Exclusive, ExclusiveTo, 97937, 0.93, &busy),
            edge(EdgeKind::Exclusive, ExclusiveTo, 97940, 0.995, &unpinned),
        ];
        let relations = RelationSet::from([(ExclusiveTo, vec![96102, 97925, 97937, 97940])]);

        assert_eq!(
            assumption(&view(96101, &relations, edges), ExclusiveTo).as_deref(),
            Some(
                "Assumption: the server completes it and 96102, 97925 together (cooccurrence; very high; 100+ characters); \
                 the server completes it and 97937 together (cooccurrence; medium; 100+ characters); \
                 the server completes it and 97940 together (cooccurrence; very high; <10 characters)"
            )
        );
    }

    #[test]
    fn explains_a_chain_implied_by_a_breadcrumb_by_the_breadcrumb() {
        let claims = [claim("handoff", NextQuestInChain, 0.66, 53)];
        let next = EdgeView {
            outcome: Outcome::Derived,
            ..edge(EdgeKind::Next, NextQuestInChain, 96392, 0.97, &claims)
        };
        let relations = RelationSet::from([(NextQuestInChain, vec![96392])]);

        assert_eq!(
            assumption(&view(96408, &relations, vec![next]), NextQuestInChain).as_deref(),
            Some("Assumption: implied by its breadcrumb to 96392 (high)")
        );
    }

    #[test]
    fn words_a_breadcrumb_from_each_side_and_absence_alone_by_the_side_it_observed() {
        // Breadcrumb 463 -> 276. With the breadcrumbs signal it leads on both sides; absence alone
        // saw 276 hidden while 463 was in the log, whichever quest the comment is on.
        let both = [claim("absence", Breadcrumbs, 0.9, 3), claim("breadcrumbs", BreadcrumbForQuestId, 0.9, 16)];
        let absence = [claim("absence", Breadcrumbs, 0.9, 3)];
        let on_breadcrumb = RelationSet::from([(BreadcrumbForQuestId, vec![276])]);
        let on_target = RelationSet::from([(Breadcrumbs, vec![463])]);
        let said = |claims: &[Claim<'static>], quest_id: QuestId, field: RelationField, target: QuestId, relations: &RelationSet| {
            let edge = edge(EdgeKind::Breadcrumb, field, target, 0.99, claims);
            assumption(&view(quest_id, relations, vec![edge]), field)
        };

        assert_eq!(
            said(&both, 463, BreadcrumbForQuestId, 276, &on_breadcrumb).as_deref(),
            Some("Assumption: optional lead-in to 276 (breadcrumbs, absence; very high; 10+ characters)")
        );
        assert_eq!(
            said(&both, 276, Breadcrumbs, 463, &on_target).as_deref(),
            Some("Assumption: 463 is an optional lead-in to it (breadcrumbs, absence; very high; 10+ characters)")
        );
        assert_eq!(
            said(&absence, 463, BreadcrumbForQuestId, 276, &on_breadcrumb).as_deref(),
            Some("Assumption: 276 left off offer lists while 463 was in the log (absence; very high; <10 characters)")
        );
        assert_eq!(
            said(&absence, 276, Breadcrumbs, 463, &on_target).as_deref(),
            Some("Assumption: 276 left off offer lists while 463 was in the log (absence; very high; <10 characters)")
        );
    }
}
