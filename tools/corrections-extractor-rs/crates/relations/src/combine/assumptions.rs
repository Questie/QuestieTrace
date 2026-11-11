//! The `-- Assumption:` comment on each field of foreverQuestRelationTraces.lua: why a written
//! relation is believed, in one line a QuestieDB maintainer can read without the review report.
//!
//! Per target it names the strongest trace observation (the claim whose signal added the most
//! log-odds), the independent characters behind it, how many trace signals agree, and the combined
//! score. Targets whose reasons differ only by id share one, so a large exclusive group stays one
//! readable line. Wowhead never appears: it is context only (see [`super::inputs`]).
//! relations-review.md has the full evidence for every relation.

use super::edges::{Claim, EdgeKind};
use crate::text::to_fixed;
use super::resolve::Outcome;
use super::views::{EdgeView, QuestView};
use crate::model::{QuestId, RelationField};
use indexmap::IndexMap;
use rustc_hash::FxHashSet;
use std::cmp::Ordering;

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

fn observation(claim: &Claim, edge: &EdgeView, quest_id: QuestId, other: &str) -> String {
    // Absence sees a breadcrumb B -> T from one side only, and the claim's field says which: B
    // hidden once T is taken (breadcrumbForQuestId) or T hidden while B is in the log (breadcrumbs).
    if edge.kind() == EdgeKind::Breadcrumb && claim.signal() == "absence" {
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

/// The claim whose signal added the most log-odds; score, then support, break ties and cover
/// edges without contributions.
fn strongest<'v>(edge: &'v EdgeView) -> Option<&'v Claim<'v>> {
    let log_odds = |claim: &Claim| {
        edge.contributions
            .iter()
            .find(|term| term.signal == claim.signal())
            .map_or(f64::NEG_INFINITY, |term| term.log_odds)
    };
    edge.claims.iter().min_by(|a, b| {
        let by_log_odds = log_odds(b).partial_cmp(&log_odds(a)).unwrap_or(Ordering::Equal);
        by_log_odds
            .then(b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal))
            .then(b.support.cmp(&a.support))
    })
}

fn characters(support: u32) -> String {
    // Support 0: only observations a signal could not pin to one independent character.
    match support {
        0 => "no independent character".to_owned(),
        1 => "1 character".to_owned(),
        n => format!("{n} characters"),
    }
}

fn reason(edge: &EdgeView, quest_id: QuestId, other: &str) -> String {
    let score = format!("score {}", to_fixed(edge.probability, 3));
    if edge.outcome == Outcome::Derived {
        return format!("implied by its breadcrumb to {other}, {score}");
    }
    let Some(claim) = strongest(edge) else { return score };
    let mut parts = vec![format!("{} ({})", observation(claim, edge, quest_id, other), characters(claim.support))];
    let signals: FxHashSet<&str> = edge.claims.iter().map(Claim::signal).collect();
    if signals.len() > 1 {
        parts.push(format!("{} signals agree", signals.len()));
    }
    parts.push(score);
    if edge.outcome == Outcome::Clique {
        parts.push("kept below the min score for its accepted exclusive group".to_owned());
    }
    parts.join(", ")
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
            context: &[],
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
            hints: vec![],
            note: None,
        }
    }

    use RelationField::{BreadcrumbForQuestId, Breadcrumbs, ExclusiveTo, NextQuestInChain, PreQuestSingle};

    #[test]
    fn names_the_signal_that_moved_the_score_most_not_the_one_with_the_highest_raw_score() {
        let claims = [
            claim("offer-set", PreQuestSingle, 0.98, 13),
            claim("handoff", PreQuestSingle, 0.97, 8),
            claim("absence", PreQuestSingle, 0.94, 1),
        ];
        let contributions = [
            Contribution {
                signal: "base",
                log_odds: -1.1,
            },
            Contribution {
                signal: "offer-set",
                log_odds: 2.4,
            },
            Contribution {
                signal: "handoff",
                log_odds: 2.7,
            },
            Contribution {
                signal: "absence",
                log_odds: -0.7,
            },
        ];
        let prerequisite = EdgeView {
            contributions: &contributions,
            ..edge(EdgeKind::Prerequisite, PreQuestSingle, 76156, 0.99523, &claims)
        };
        let relations = RelationSet::from([(PreQuestSingle, vec![76156])]);

        assert_eq!(
            assumption(&view(76160, &relations, vec![prerequisite]), PreQuestSingle).as_deref(),
            Some("Assumption: offered right after 76156 was turned in (8 characters), 3 signals agree, score 0.995")
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
            edge(EdgeKind::Prerequisite, PreQuestSingle, 79007, 0.96, &claims),
            near_miss,
        ];
        let relations = RelationSet::from([(PreQuestSingle, vec![79007, 79008])]);
        let view = view(79192, &relations, edges);

        assert_eq!(
            assumption(&view, PreQuestSingle).as_deref(),
            Some(
                "Assumption: everyone who took it had done 79007 (1 character), score 0.960; \
                 everyone who took it had done 79008 (1 character), score 0.970"
            )
        );
        assert_eq!(assumption(&view, NextQuestInChain), None);
    }

    #[test]
    fn merges_targets_whose_reasons_differ_only_by_id() {
        let busy = [claim("cooccurrence", ExclusiveTo, 0.9, 171)];
        let unpinned = [claim("cooccurrence", ExclusiveTo, 0.9, 0)];
        let edges = vec![
            edge(EdgeKind::Exclusive, ExclusiveTo, 96102, 0.993, &busy),
            edge(EdgeKind::Exclusive, ExclusiveTo, 97925, 0.993, &busy),
            edge(EdgeKind::Exclusive, ExclusiveTo, 97937, 0.993, &unpinned),
        ];
        let relations = RelationSet::from([(ExclusiveTo, vec![96102, 97925, 97937])]);

        assert_eq!(
            assumption(&view(96101, &relations, edges), ExclusiveTo).as_deref(),
            Some(
                "Assumption: the server completes it and 96102, 97925 together (171 characters), score 0.993; \
                 the server completes it and 97937 together (no independent character), score 0.993"
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
            Some("Assumption: implied by its breadcrumb to 96392, score 0.970")
        );
    }

    #[test]
    fn words_a_breadcrumb_from_each_side_and_absence_by_the_side_it_observed() {
        // Breadcrumb 463 -> 276. Absence saw 276 hidden while 463 was in the log, whichever quest the comment is on.
        let claims = [claim("breadcrumbs", BreadcrumbForQuestId, 0.9, 16), claim("absence", Breadcrumbs, 0.9, 3)];
        let breadcrumbs_first = [Contribution {
            signal: "breadcrumbs",
            log_odds: 2.0,
        }];
        let absence_first = [Contribution {
            signal: "absence",
            log_odds: 2.0,
        }];
        let from_breadcrumb = EdgeView {
            contributions: &breadcrumbs_first,
            ..edge(EdgeKind::Breadcrumb, BreadcrumbForQuestId, 276, 0.99, &claims)
        };
        let from_target = EdgeView {
            contributions: &absence_first,
            ..edge(EdgeKind::Breadcrumb, Breadcrumbs, 463, 0.99, &claims)
        };
        let on_breadcrumb = RelationSet::from([(BreadcrumbForQuestId, vec![276])]);
        let on_target = RelationSet::from([(Breadcrumbs, vec![463])]);

        assert_eq!(
            assumption(&view(463, &on_breadcrumb, vec![from_breadcrumb]), BreadcrumbForQuestId).as_deref(),
            Some("Assumption: optional lead-in to 276 (16 characters), 2 signals agree, score 0.990")
        );
        assert_eq!(
            assumption(&view(276, &on_target, vec![from_target]), Breadcrumbs).as_deref(),
            Some("Assumption: 276 left off offer lists while 463 was in the log (3 characters), 2 signals agree, score 0.990")
        );
    }
}
