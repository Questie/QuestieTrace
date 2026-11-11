//! Merges per-page claims (`interpret`) into one [`RelationCandidate`] per (quest, field, target).
//!
//! Every quest in a Series shows the whole Series, so one edge is usually claimed by several
//! pages. Those pages render the same Wowhead data, so they are witnesses, not independent
//! observations: `support` counts pages showing the edge, `contradict` counts pages whose Series
//! shows both quests but not directly one after the other.

use super::interpret::{Claim, Surface};
use super::parse::PageEnvironment;
use super::scores::{base_score, ClaimKey, Provenance};
use crate::model::{Evidence, QuestId, RelationCandidate};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;

const MAX_EVIDENCE: usize = 5;

/// A claim with what is known about the page that showed it.
#[derive(Clone, Debug, PartialEq)]
pub struct PageClaim {
    pub claim: Claim,
    pub provenance: Provenance,
    pub environment: PageEnvironment,
    /// envChange.status of the page's own quest.
    pub env_change: Option<String>,
}

impl PageClaim {
    fn score(&self) -> f64 {
        base_score(self.claim.surface, self.claim.field, self.claim.shape, self.provenance)
    }
}

/// Page id -> (quest id -> Series row position) for every page with a Series. Used to count
/// contradicting pages.
pub type SeriesPositions = BTreeMap<QuestId, FxHashMap<QuestId, usize>>;

/// A candidate with the provenance of the claim it was scored by.
#[derive(Clone, Debug, PartialEq)]
pub struct WowheadCandidate {
    pub candidate: RelationCandidate,
    pub provenance: Provenance,
}

/// Candidates per surface, each list by quest, field name and target.
pub fn aggregate_claims(claims: Vec<PageClaim>, series_positions: &SeriesPositions) -> BTreeMap<Surface, Vec<WowheadCandidate>> {
    // Within a group, claims stay in page order: ties below go to the first page.
    let mut groups: BTreeMap<ClaimKey, Vec<PageClaim>> = BTreeMap::new();
    for claim in claims {
        groups.entry(ClaimKey::of(&claim.claim)).or_default().push(claim);
    }

    let pages_by_quest = index_pages_by_quest(series_positions);
    let mut by_surface: BTreeMap<Surface, Vec<WowheadCandidate>> = BTreeMap::new();
    for (key, group) in &groups {
        by_surface
            .entry(key.surface)
            .or_default()
            .push(to_candidate(group, series_positions, &pages_by_quest));
    }
    for list in by_surface.values_mut() {
        // By field name, as the TypeScript pipeline sorts (its localeCompare orders these
        // lowercase-first camelCase names like plain byte order does).
        list.sort_by(|a, b| {
            let (a, b) = (&a.candidate, &b.candidate);
            (a.quest_id, a.field.name(), a.target).cmp(&(b.quest_id, b.field.name(), b.target))
        });
    }
    by_surface
}

fn to_candidate(group: &[PageClaim], series_positions: &SeriesPositions, pages_by_quest: &FxHashMap<QuestId, Vec<QuestId>>) -> WowheadCandidate {
    let first = &group[0].claim;
    // The most trusted claim: highest score, then most trusted provenance, then lowest page id.
    // `min_by` keeps the first of equals, like the stable sort it replaces.
    let best = group
        .iter()
        .min_by(|a, b| {
            b.score()
                .total_cmp(&a.score())
                .then(a.provenance.cmp(&b.provenance))
                .then(a.claim.page_id.cmp(&b.claim.page_id))
        })
        .expect("a group has a claim");

    let support = group.iter().map(|claim| claim.claim.page_id).collect::<FxHashSet<_>>().len();
    let contradict = best
        .claim
        .adjacency
        .map_or(0, |adjacency| contradicting_pages(adjacency, series_positions, pages_by_quest));
    // Pages that show the two quests in a different order or with steps between them dilute the edge.
    let score = round(best.score() * (support as f64 / (support + contradict) as f64));

    // Forever pages first, then by page id; one example per page.
    let mut freshest: Vec<&PageClaim> = group.iter().collect();
    freshest.sort_by_key(|claim| (claim.environment != PageEnvironment::Forever, claim.claim.page_id));
    let mut seen = FxHashSet::default();
    let evidence = freshest
        .into_iter()
        .filter(|claim| seen.insert(claim.claim.page_id))
        .take(MAX_EVIDENCE)
        .map(|claim| Evidence {
            reference: format!("wowhead:quest/{}", claim.claim.page_id),
            t: None,
            text: format!("{}: {}", describe_page(claim), claim.claim.text),
        })
        .collect();

    WowheadCandidate {
        candidate: RelationCandidate {
            quest_id: first.quest_id,
            field: first.field,
            target: first.target,
            score,
            support: support as u32,
            contradict: contradict as u32,
            evidence,
            // Machine-readable tags for review and per-provenance scoring.
            note: Some(format!(
                "surface={} field={} shape={} provenance={}",
                first.surface.name(),
                first.field,
                best.claim.shape.name(),
                best.provenance.name()
            )),
        },
        provenance: best.provenance,
    }
}

fn contradicting_pages((before, after): (QuestId, QuestId), series_positions: &SeriesPositions, pages_by_quest: &FxHashMap<QuestId, Vec<QuestId>>) -> usize {
    let pages = pages_by_quest.get(&before).map_or(&[][..], Vec::as_slice);
    pages
        .iter()
        .filter(|page_id| {
            let positions = &series_positions[page_id];
            match (positions.get(&before), positions.get(&after)) {
                (Some(&a), Some(&b)) => b != a + 1,
                _ => false,
            }
        })
        .count()
}

fn index_pages_by_quest(series_positions: &SeriesPositions) -> FxHashMap<QuestId, Vec<QuestId>> {
    let mut index: FxHashMap<QuestId, Vec<QuestId>> = FxHashMap::default();
    for (&page_id, positions) in series_positions {
        for &quest_id in positions.keys() {
            index.entry(quest_id).or_default().push(page_id);
        }
    }
    index
}

fn describe_page(claim: &PageClaim) -> String {
    let environment = match claim.environment {
        PageEnvironment::Forever => "Forever page",
        PageEnvironment::Era => "inherited Era page",
        PageEnvironment::Unknown => "page",
    };
    let status = match claim.env_change.as_deref() {
        Some(status) if !status.is_empty() => format!(", envChange {status}"),
        _ => String::new(),
    };
    let edge = match claim.provenance {
        Provenance::ForeverNewEdge => ", edge not on any Era page",
        Provenance::ForeverEraEdge => ", edge also on Era pages",
        Provenance::EraPage => "",
    };
    format!("{environment} {}{status}{edge}", claim.claim.page_id)
}

/// To two decimals.
fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::super::interpret::ClaimShape;
    use super::*;
    use crate::model::RelationField;

    fn series_claim(page_id: QuestId) -> PageClaim {
        PageClaim {
            claim: Claim {
                surface: Surface::Series,
                quest_id: 2,
                field: RelationField::PreQuestSingle,
                target: 1,
                shape: ClaimShape::Single,
                page_id,
                adjacency: Some((1, 2)),
                text: "Series row 1 [1] -> row 2 [2]".into(),
            },
            provenance: Provenance::ForeverNewEdge,
            environment: PageEnvironment::Forever,
            env_change: None,
        }
    }

    fn positions(pages: &[(QuestId, &[QuestId])]) -> SeriesPositions {
        pages
            .iter()
            .map(|&(page_id, order)| (page_id, order.iter().enumerate().map(|(row, &quest_id)| (quest_id, row)).collect()))
            .collect()
    }

    #[test]
    fn merges_pages_showing_one_edge_counts_pages_that_order_the_quests_differently_and_dilutes_the_score() {
        // Pages 1 and 2 show 1 -> 2; page 3 shows 1 -> 9 -> 2; page 4 does not show quest 2 at all.
        let series = positions(&[(1, &[1, 2]), (2, &[1, 2]), (3, &[1, 9, 2]), (4, &[1, 9])]);
        let candidates = &aggregate_claims(vec![series_claim(1), series_claim(2)], &series)[&Surface::Series];
        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0].candidate;
        assert_eq!((candidate.quest_id, candidate.field, candidate.target), (2, RelationField::PreQuestSingle, 1));
        assert_eq!((candidate.support, candidate.contradict), (2, 1));
        // forever-new-edge Series single scores 0.85; 0.85 * 2/3 = 0.5667.
        assert_eq!(candidate.score, 0.57);
    }

    #[test]
    fn scores_by_the_most_trusted_claim_and_lists_forever_pages_first_in_the_evidence() {
        let mut era = series_claim(5);
        era.environment = PageEnvironment::Era;
        era.provenance = Provenance::EraPage;
        era.claim.shape = ClaimShape::Variants;
        let mut forever = series_claim(9);
        forever.provenance = Provenance::ForeverEraEdge;
        forever.env_change = Some("updated".into());

        let series = positions(&[(5, &[1, 2]), (9, &[1, 2])]);
        let result = aggregate_claims(vec![era, forever], &series);
        let WowheadCandidate { candidate, provenance } = &result[&Surface::Series][0];
        assert_eq!(candidate.score, 0.95);
        assert_eq!(*provenance, Provenance::ForeverEraEdge);
        assert_eq!(
            candidate.note.as_deref(),
            Some("surface=series field=preQuestSingle shape=single provenance=forever-era-edge")
        );
        let evidence: Vec<(&str, &str)> = candidate.evidence.iter().map(|e| (e.reference.as_str(), e.text.as_str())).collect();
        assert_eq!(
            evidence,
            [
                (
                    "wowhead:quest/9",
                    "Forever page 9, envChange updated, edge also on Era pages: Series row 1 [1] -> row 2 [2]"
                ),
                ("wowhead:quest/5", "inherited Era page 5: Series row 1 [1] -> row 2 [2]"),
            ]
        );
    }

    #[test]
    fn candidates_are_ordered_by_quest_then_field_name_then_target() {
        let claim = |quest_id, field, target| {
            let mut claim = series_claim(1);
            (claim.claim.quest_id, claim.claim.field, claim.claim.target, claim.claim.adjacency) = (quest_id, field, target, None);
            claim
        };
        let claims = vec![
            claim(2, RelationField::PreQuestSingle, 7),
            claim(2, RelationField::NextQuestInChain, 9),
            claim(1, RelationField::PreQuestSingle, 3),
            claim(2, RelationField::PreQuestSingle, 5),
        ];
        let order: Vec<(QuestId, RelationField, QuestId)> = aggregate_claims(claims, &SeriesPositions::new())[&Surface::Series]
            .iter()
            .map(|c| (c.candidate.quest_id, c.candidate.field, c.candidate.target))
            .collect();
        // nextQuestInChain before preQuestSingle by name, although the field enum lists it later.
        assert_eq!(
            order,
            [
                (1, RelationField::PreQuestSingle, 3),
                (2, RelationField::NextQuestInChain, 9),
                (2, RelationField::PreQuestSingle, 5),
                (2, RelationField::PreQuestSingle, 7)
            ]
        );
    }
}
