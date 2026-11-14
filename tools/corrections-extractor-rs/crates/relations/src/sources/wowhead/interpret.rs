//! Turns the relation surfaces of one parsed Wowhead quest page (`parse`) into claims about Questie
//! relation fields (semantics on [`RelationField`]).
//!
//! Mapping, per surface:
//!
//! ```text
//! series                Adjacent non-empty rows P -> N. For each quest b in N, its compatible
//!                       members of P become b.preQuestSingle. Wowhead does not say AND or OR for
//!                       several different quests in one row; Questie's data almost never uses
//!                       preQuestGroup for them, so they are preQuestSingle with shape "group".
//!                       For each a in P with exactly one compatible quest b in N:
//!                       a.nextQuestInChain = b.
//!                       When P is the first row and its quests have no objectives, a is often a
//!                       breadcrumb rather than a prerequisite (shape "lead-in"): such edges also
//!                       yield a.breadcrumbForQuestId = b and b.breadcrumbs = a.
//! requires              On page Q: Q.preQuestSingle, or preQuestGroup when the listed quests (other
//!                       than Q itself) have different names.
//! requires-any          On page Q: Q.preQuestSingle for each listed quest except Q itself.
//! requires-in-progress  On page Q: Q.parentQuest (Q needs the listed quest in the log).
//! unlocks               On page Q: X.preQuestSingle = Q for each listed X except Q itself.
//! disables              On page Q: X.exclusiveTo = Q for each listed X except Q itself
//!                       (taking Q makes X unavailable; Questie needs that edge on X).
//! storyline             Adjacent steps (runs of same-name quests are one step): next.preQuestSingle = prev.
//! ```
//!
//! "Compatible" drops pairs no character can do both of (opposite factions, disjoint race or class
//! masks). Series rows merge parallel race/class chains, so this matters a lot there.
//! Scores live in `scores`, keyed by surface, shape and provenance.

use super::parse::{MarkupPanel, QuestLookup, QuestPageFacts, SeriesMember, Side};
use crate::model::{QuestId, RelationField};
use serde::Serialize;

/// A Wowhead page surface; each is its own candidates file, `wowhead-<surface>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    Series,
    Requires,
    RequiresAny,
    RequiresInProgress,
    Unlocks,
    Disables,
    Storyline,
}

impl Surface {
    /// In the order the candidate files are written.
    pub const ALL: [Surface; 7] = [
        Surface::Series,
        Surface::Requires,
        Surface::RequiresAny,
        Surface::RequiresInProgress,
        Surface::Unlocks,
        Surface::Disables,
        Surface::Storyline,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Surface::Series => "series",
            Surface::Requires => "requires",
            Surface::RequiresAny => "requires-any",
            Surface::RequiresInProgress => "requires-in-progress",
            Surface::Unlocks => "unlocks",
            Surface::Disables => "disables",
            Surface::Storyline => "storyline",
        }
    }

    /// The markup panel this surface reads, for the panel surfaces.
    pub fn panel(self) -> Option<MarkupPanel> {
        match self {
            Surface::Requires => Some(MarkupPanel::Requires),
            Surface::RequiresAny => Some(MarkupPanel::RequiresAny),
            Surface::RequiresInProgress => Some(MarkupPanel::RequiresInProgress),
            Surface::Unlocks => Some(MarkupPanel::Unlocks),
            Surface::Disables => Some(MarkupPanel::Disables),
            Surface::Series | Surface::Storyline => None,
        }
    }
}

/// How the claim's source structure looked, which decides how far to trust its field mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClaimShape {
    /// Exactly one quest on the other side of the edge.
    Single,
    /// Several same-name quests (race/faction/class variants of one step).
    Variants,
    /// Several different quests (AND or OR is not shown).
    Group,
    /// A list that includes the page's own quest (repeatable turn-in groups).
    WithSelf,
    /// Series first-row quests without objectives ("go talk to X"), which are as often breadcrumbs
    /// as prerequisites.
    LeadIn,
}

impl ClaimShape {
    pub fn name(self) -> &'static str {
        match self {
            ClaimShape::Single => "single",
            ClaimShape::Variants => "variants",
            ClaimShape::Group => "group",
            ClaimShape::WithSelf => "with-self",
            ClaimShape::LeadIn => "lead-in",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Claim {
    pub surface: Surface,
    pub quest_id: QuestId,
    pub field: RelationField,
    pub target: QuestId,
    pub shape: ClaimShape,
    /// The page that showed it.
    pub page_id: QuestId,
    /// For series claims: the ordered pair (before, after) of adjacent rows the claim was read from.
    pub adjacency: Option<(QuestId, QuestId)>,
    /// One line a reviewer can check against the page.
    pub text: String,
}

/// `quests`: facts merged across all pages; a page alone often lacks its neighbours' restrictions.
pub fn claims_from_page(page: &QuestPageFacts, quests: &QuestLookup) -> Vec<Claim> {
    let context = Context { page, quests };
    let mut claims = context.series_claims();
    claims.extend(context.storyline_claims());
    for surface in Surface::ALL {
        let Some(panel) = surface.panel() else { continue };
        match page.panels.get(&panel) {
            Some(ids) if !ids.is_empty() => claims.extend(context.panel_claims(surface, panel, ids)),
            _ => {}
        }
    }
    claims
}

struct Context<'a> {
    page: &'a QuestPageFacts,
    quests: &'a QuestLookup,
}

impl Context<'_> {
    // ----------------------------------------------------------------------- Series

    fn series_claims(&self) -> Vec<Claim> {
        let mut claims = Vec::new();
        let rows = &self.page.series;
        for (i, pair) in rows.windows(2).enumerate() {
            let (prev, next) = (&pair[0].members, &pair[1].members);
            if prev.is_empty() || next.is_empty() {
                continue;
            }
            let first_row = rows[..i].iter().all(|row| row.members.is_empty());
            let text = format!(
                "Series row {} [{}] -> row {} [{}]",
                pair[0].index,
                self.describe_members(prev),
                pair[1].index,
                self.describe_members(next)
            );
            let claim = |quest_id, field, target, shape, adjacency| Claim {
                surface: Surface::Series,
                quest_id,
                field,
                target,
                shape,
                page_id: self.page.quest_id,
                adjacency: Some(adjacency),
                text: text.clone(),
            };

            for b in next.iter().map(|member| member.quest_id) {
                let before: Vec<QuestId> = prev.iter().map(|a| a.quest_id).filter(|&a| a != b && self.compatible(a, b)).collect();
                if before.is_empty() {
                    continue;
                }
                let lead_in = first_row && before.iter().all(|&a| self.has_no_objectives(a));
                let shape = if lead_in {
                    ClaimShape::LeadIn
                } else if before.len() == 1 {
                    ClaimShape::Single
                } else if self.same_name(&before) {
                    ClaimShape::Variants
                } else {
                    ClaimShape::Group
                };
                for &a in &before {
                    claims.push(claim(b, RelationField::PreQuestSingle, a, shape, (a, b)));
                }
            }

            for a in prev.iter().map(|member| member.quest_id) {
                let after: Vec<QuestId> = next.iter().map(|b| b.quest_id).filter(|&b| b != a && self.compatible(a, b)).collect();
                let [b] = after[..] else { continue };
                let lead_in = first_row && self.has_no_objectives(a);
                let shape = if lead_in { ClaimShape::LeadIn } else { ClaimShape::Single };
                claims.push(claim(a, RelationField::NextQuestInChain, b, shape, (a, b)));
                if lead_in {
                    claims.push(claim(a, RelationField::BreadcrumbForQuestId, b, ClaimShape::LeadIn, (a, b)));
                    claims.push(claim(b, RelationField::Breadcrumbs, a, ClaimShape::LeadIn, (a, b)));
                }
            }
        }
        claims
    }

    fn describe_members(&self, members: &[SeriesMember]) -> String {
        members.iter().map(|member| self.quest_label(member.quest_id)).collect::<Vec<_>>().join(" | ")
    }

    // ----------------------------------------------------------------------- Storyline

    fn storyline_claims(&self) -> Vec<Claim> {
        let Some(storyline) = &self.page.storyline else {
            return Vec::new();
        };

        // Consecutive same-name quests are variants of one step (e.g. 92750/92751), not a sequence.
        let mut steps: Vec<Vec<QuestId>> = Vec::new();
        for &id in &storyline.quest_ids {
            match steps.last_mut() {
                Some(last) if self.same_name(&[last[0], id]) => last.push(id),
                _ => steps.push(vec![id]),
            }
        }

        let describe = |step: &[QuestId]| step.iter().map(|&id| self.quest_label(id)).collect::<Vec<_>>().join(" | ");
        let mut claims = Vec::new();
        for (i, pair) in steps.windows(2).enumerate() {
            let text = format!(
                "Storyline {} \"{}\" step {} [{}] -> step {} [{}]",
                storyline.id,
                storyline.name,
                i + 1,
                describe(&pair[0]),
                i + 2,
                describe(&pair[1])
            );
            for &b in &pair[1] {
                let before: Vec<QuestId> = pair[0].iter().copied().filter(|&a| self.compatible(a, b)).collect();
                let shape = if before.len() == 1 { ClaimShape::Single } else { ClaimShape::Variants };
                for a in before {
                    claims.push(Claim {
                        surface: Surface::Storyline,
                        quest_id: b,
                        field: RelationField::PreQuestSingle,
                        target: a,
                        shape,
                        page_id: self.page.quest_id,
                        adjacency: None,
                        text: text.clone(),
                    });
                }
            }
        }
        claims
    }

    // ----------------------------------------------------------------------- Markup panels

    fn panel_claims(&self, surface: Surface, panel: MarkupPanel, ids: &[QuestId]) -> Vec<Claim> {
        let own = self.page.quest_id;
        let others: Vec<QuestId> = ids.iter().copied().filter(|&id| id != own).collect();
        let with_self = others.len() < ids.len();
        let different_quests = others.len() > 1 && !self.same_name(&others);
        let shape = if with_self {
            ClaimShape::WithSelf
        } else if others.len() == 1 {
            ClaimShape::Single
        } else if different_quests {
            ClaimShape::Group
        } else {
            ClaimShape::Variants
        };
        let labels: Vec<String> = ids.iter().map(|&id| self.quest_label(id)).collect();
        let text = format!("{}: {}", panel.heading(), labels.join(", "));
        let claim = |quest_id, field, target| Claim {
            surface,
            quest_id,
            field,
            target,
            shape,
            page_id: own,
            adjacency: None,
            text: text.clone(),
        };

        others
            .into_iter()
            .map(|id| match panel {
                MarkupPanel::Requires if different_quests => claim(own, RelationField::PreQuestGroup, id),
                MarkupPanel::Requires | MarkupPanel::RequiresAny => claim(own, RelationField::PreQuestSingle, id),
                MarkupPanel::RequiresInProgress => claim(own, RelationField::ParentQuest, id),
                MarkupPanel::Unlocks => claim(id, RelationField::PreQuestSingle, own),
                MarkupPanel::Disables => claim(id, RelationField::ExclusiveTo, own),
            })
            .collect()
    }

    // ----------------------------------------------------------------------- Helpers

    /// False when no character could do both quests: opposite factions, or disjoint race or class masks.
    fn compatible(&self, a: QuestId, b: QuestId) -> bool {
        let (Some(qa), Some(qb)) = (self.quests.get(&a), self.quests.get(&b)) else {
            return true;
        };
        let opposite_factions = matches!(
            (qa.side, qb.side),
            (Some(Side::Alliance), Some(Side::Horde)) | (Some(Side::Horde), Some(Side::Alliance))
        );
        let disjoint = |x: Option<u64>, y: Option<u64>| matches!((x, y), (Some(x), Some(y)) if x & y == 0);
        !opposite_factions && !disjoint(qa.races, qb.races) && !disjoint(qa.classes, qb.classes)
    }

    /// Known to have no objectives (unknown is not enough for a lead-in).
    fn has_no_objectives(&self, id: QuestId) -> bool {
        self.quests.get(&id).is_some_and(|quest| quest.has_objectives == Some(false))
    }

    /// True when every quest has a known name and all names are equal.
    fn same_name(&self, ids: &[QuestId]) -> bool {
        let mut names = ids.iter().map(|id| self.quests.get(id).and_then(|quest| quest.name.as_deref()));
        let Some(first) = names.next() else { return true };
        first.is_some() && names.all(|name| name == first)
    }

    fn quest_label(&self, id: QuestId) -> String {
        match self.quests.get(&id).and_then(|quest| quest.name.as_deref()) {
            Some(name) => format!("{id} {name}"),
            None => id.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse::{PageEnvironment, QuestRecord, SeriesRow, Storyline};
    use super::*;
    use std::collections::BTreeMap;

    fn page(quest_id: QuestId) -> QuestPageFacts {
        QuestPageFacts {
            quest_id,
            environment: PageEnvironment::Forever,
            env_change: None,
            series: vec![],
            panels: BTreeMap::new(),
            storyline: None,
            quests: QuestLookup::new(),
        }
    }

    /// Series rows from quest id lists; the page's own quest is marked current.
    fn series(quest_id: QuestId, rows: &[&[QuestId]]) -> Vec<SeriesRow> {
        rows.iter()
            .enumerate()
            .map(|(i, ids)| SeriesRow {
                index: i as i64 + 1,
                members: ids
                    .iter()
                    .map(|&id| SeriesMember {
                        quest_id: id,
                        current: id == quest_id,
                    })
                    .collect(),
                unparsed: false,
            })
            .collect()
    }

    fn series_page(quest_id: QuestId, rows: &[&[QuestId]]) -> QuestPageFacts {
        QuestPageFacts {
            series: series(quest_id, rows),
            ..page(quest_id)
        }
    }

    fn named(name: &str) -> QuestRecord {
        QuestRecord {
            name: Some(name.into()),
            ..QuestRecord::default()
        }
    }

    fn objectives(has: bool) -> QuestRecord {
        QuestRecord {
            has_objectives: Some(has),
            ..QuestRecord::default()
        }
    }

    fn lookup(records: impl IntoIterator<Item = (QuestId, QuestRecord)>) -> QuestLookup {
        records.into_iter().collect()
    }

    fn edges(claims: &[Claim]) -> Vec<String> {
        let mut edges: Vec<String> = claims
            .iter()
            .map(|c| format!("{}.{}={} ({})", c.quest_id, c.field, c.target, c.shape.name()))
            .collect();
        edges.sort();
        edges
    }

    fn prerequisite_edges(claims: &[Claim]) -> Vec<String> {
        let prerequisites: Vec<Claim> = claims.iter().filter(|c| c.field == RelationField::PreQuestSingle).cloned().collect();
        edges(&prerequisites)
    }

    #[test]
    fn a_linear_series_links_adjacent_rows_only() {
        let claims = claims_from_page(&series_page(2, &[&[1], &[2], &[3]]), &QuestLookup::new());
        assert_eq!(
            edges(&claims),
            [
                "1.nextQuestInChain=2 (single)",
                "2.nextQuestInChain=3 (single)",
                "2.preQuestSingle=1 (single)",
                "3.preQuestSingle=2 (single)"
            ]
        );
        assert_eq!(claims[0].text, "Series row 1 [1] -> row 2 [2]");
    }

    #[test]
    fn a_series_does_not_link_across_an_empty_row() {
        assert_eq!(claims_from_page(&series_page(1, &[&[1], &[], &[3]]), &QuestLookup::new()), vec![]);
    }

    #[test]
    fn a_series_drops_pairs_no_character_can_do_both_of() {
        // Wowhead's Taming the Beast row merges Night Elf quest 6063 with Forever-race quest 94013.
        let hunter = |name: &str, races: u64| QuestRecord {
            races: Some(races),
            classes: Some(4),
            ..named(name)
        };
        let side = |side| QuestRecord {
            side: Some(side),
            ..QuestRecord::default()
        };
        let quests = lookup([
            (6071, hunter("The Hunter's Path", 8)),
            (94007, hunter("Taming the Beast", 12884901888)),
            (6063, hunter("Taming the Beast", 8)),
            (94013, hunter("Taming the Beast", 12884901888)),
            (500, side(Side::Alliance)),
            (501, side(Side::Horde)),
        ]);
        assert_eq!(
            edges(&claims_from_page(&series_page(6063, &[&[6071, 94007], &[6063, 94013]]), &quests)),
            [
                "6063.preQuestSingle=6071 (single)",
                "6071.nextQuestInChain=6063 (single)",
                "94007.nextQuestInChain=94013 (single)",
                "94013.preQuestSingle=94007 (single)"
            ]
        );
        assert_eq!(claims_from_page(&series_page(500, &[&[500], &[501]]), &quests), vec![]);
    }

    #[test]
    fn several_same_name_quests_are_variants_and_several_different_quests_a_group() {
        let quests = lookup([(10, named("Call of Fire")), (11, named("Call of Fire")), (20, named("A")), (21, named("B"))]);
        assert_eq!(
            prerequisite_edges(&claims_from_page(&series_page(12, &[&[10, 11], &[12]]), &quests)),
            ["12.preQuestSingle=10 (variants)", "12.preQuestSingle=11 (variants)"]
        );
        assert_eq!(
            prerequisite_edges(&claims_from_page(&series_page(22, &[&[20, 21], &[22]]), &quests)),
            ["22.preQuestSingle=20 (group)", "22.preQuestSingle=21 (group)"]
        );
    }

    #[test]
    fn no_next_quest_in_chain_when_a_quest_leads_to_several() {
        let claims = claims_from_page(&series_page(1, &[&[1], &[2, 3]]), &QuestLookup::new());
        assert!(claims.iter().all(|claim| claim.field != RelationField::NextQuestInChain));
        assert_eq!(claims.len(), 2);
    }

    #[test]
    fn first_row_quests_without_objectives_are_lead_ins_with_breadcrumb_claims() {
        // 239 Westbrook Garrison Needs Help! is a breadcrumb into 11 Riverpaw Gnoll Bounty.
        let quests = lookup([(239, objectives(false)), (11, objectives(true)), (12, objectives(false))]);
        assert_eq!(
            edges(&claims_from_page(&series_page(11, &[&[239], &[11], &[12]]), &quests)),
            [
                "11.breadcrumbs=239 (lead-in)",
                "11.nextQuestInChain=12 (single)",
                "11.preQuestSingle=239 (lead-in)",
                "12.preQuestSingle=11 (single)",
                "239.breadcrumbForQuestId=11 (lead-in)",
                "239.nextQuestInChain=11 (lead-in)"
            ]
        );
    }

    #[test]
    fn a_first_row_is_no_lead_in_when_objectives_are_unknown_or_present() {
        let quests = lookup([(1, objectives(true))]);
        assert_eq!(
            edges(&claims_from_page(&series_page(2, &[&[1], &[2]]), &quests)),
            ["1.nextQuestInChain=2 (single)", "2.preQuestSingle=1 (single)"]
        );
        assert_eq!(
            edges(&claims_from_page(&series_page(2, &[&[3], &[2]]), &QuestLookup::new())),
            ["2.preQuestSingle=3 (single)", "3.nextQuestInChain=2 (single)"]
        );
    }

    fn panel_page(quest_id: QuestId, panels: &[(MarkupPanel, &[QuestId])]) -> QuestPageFacts {
        QuestPageFacts {
            environment: PageEnvironment::Era,
            panels: panels.iter().map(|&(panel, ids)| (panel, ids.to_vec())).collect(),
            ..page(quest_id)
        }
    }

    #[test]
    fn requires_panels_sit_on_the_pages_quest_unlocks_and_disables_on_the_listed_quests() {
        let facts = panel_page(
            100,
            &[
                (MarkupPanel::Requires, &[90]),
                (MarkupPanel::RequiresInProgress, &[91]),
                (MarkupPanel::Unlocks, &[110]),
                (MarkupPanel::Disables, &[120]),
            ],
        );
        assert_eq!(
            edges(&claims_from_page(&facts, &QuestLookup::new())),
            [
                "100.parentQuest=91 (single)",
                "100.preQuestSingle=90 (single)",
                "110.preQuestSingle=100 (single)",
                "120.exclusiveTo=100 (single)"
            ]
        );
    }

    #[test]
    fn self_including_lists_drop_the_pages_own_quest_and_are_marked() {
        let facts = panel_page(7886, &[(MarkupPanel::RequiresAny, &[7886, 7887, 7788])]);
        let claims = claims_from_page(&facts, &QuestLookup::new());
        assert_eq!(edges(&claims), ["7886.preQuestSingle=7788 (with-self)", "7886.preQuestSingle=7887 (with-self)"]);
        assert_eq!(claims[0].text, "Requires Any: 7886, 7887, 7788");
    }

    #[test]
    fn a_requires_list_of_different_quests_is_a_pre_quest_group_also_when_it_lists_the_pages_quest() {
        let quests = lookup([(1, named("A")), (2, named("B")), (5, named("C"))]);
        let facts = panel_page(5, &[(MarkupPanel::Requires, &[1, 2])]);
        assert_eq!(
            edges(&claims_from_page(&facts, &quests)),
            ["5.preQuestGroup=1 (group)", "5.preQuestGroup=2 (group)"]
        );
        let with_self = panel_page(5, &[(MarkupPanel::Requires, &[5, 1, 2])]);
        assert_eq!(
            edges(&claims_from_page(&with_self, &quests)),
            ["5.preQuestGroup=1 (with-self)", "5.preQuestGroup=2 (with-self)"]
        );
    }

    #[test]
    fn storyline_runs_of_same_name_quests_are_one_step_linked_to_both_neighbours() {
        let quests = lookup([
            (92749, named("A Dynamite Plan")),
            (92750, named("Detonation at a Distance")),
            (92751, named("Detonation at a Distance")),
            (92752, named("Explosive Consultation")),
        ]);
        let facts = QuestPageFacts {
            storyline: Some(Storyline {
                id: 6026,
                name: "Toxic Soil".into(),
                quest_ids: vec![92749, 92750, 92751, 92752],
            }),
            ..page(92749)
        };
        let claims = claims_from_page(&facts, &quests);
        assert_eq!(
            edges(&claims),
            [
                "92750.preQuestSingle=92749 (single)",
                "92751.preQuestSingle=92749 (single)",
                "92752.preQuestSingle=92750 (variants)",
                "92752.preQuestSingle=92751 (variants)"
            ]
        );
        assert_eq!(
            claims[0].text,
            "Storyline 6026 \"Toxic Soil\" step 1 [92749 A Dynamite Plan] -> step 2 [92750 Detonation at a Distance | 92751 Detonation at a Distance]"
        );
    }
}
