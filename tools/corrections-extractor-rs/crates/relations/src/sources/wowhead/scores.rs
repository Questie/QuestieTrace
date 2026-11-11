//! Provenance and base scores for Wowhead claims.
//!
//! Wowhead is untrusted: combine shows these claims to reviewers as context only and never lets
//! them decide a relation. The scores order claims for that review, nothing more.
//!
//! A base score is the measured precision of that kind of claim against groundtruth.json, rounded
//! down and shrunk for small samples. That measures agreement with Questie's current data (authored
//! Forever fixes, inherited Classic relations), not with what Forever servers actually do, and the
//! agreement is partly circular: Questie's existing relations were partly scraped from these same
//! Series rows. Re-measure after changing the parser or mapping: `relations source:wowhead`, then
//! `relations score wowhead-<surface>`; the candidate `note` carries field, shape and provenance
//! for finer splits.

use super::interpret::{Claim, ClaimShape, Surface};
use super::parse::PageEnvironment;
use crate::model::{QuestId, RelationField};
use rustc_hash::FxHashSet;
use serde::Serialize;

/// Where a claim comes from. Declaration order is trust order, most trusted first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    /// On a page fetched from Wowhead's Forever environment, and the Era cache does not produce
    /// the same claim. The only Wowhead evidence about Forever changes.
    ForeverNewEdge,
    /// On a Forever page, and Era pages produce the same claim (inherited content).
    ForeverEraEdge,
    /// Only on Era pages that scraper-questie copied into the Forever cache and never refetched.
    /// Era data, not Forever evidence.
    EraPage,
}

impl Provenance {
    pub fn name(self) -> &'static str {
        match self {
            Provenance::ForeverNewEdge => "forever-new-edge",
            Provenance::ForeverEraEdge => "forever-era-edge",
            Provenance::EraPage => "era-page",
        }
    }
}

/// What makes two claims the same edge, whichever page showed it and in which shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimKey {
    pub surface: Surface,
    pub quest_id: QuestId,
    pub field: RelationField,
    pub target: QuestId,
}

impl ClaimKey {
    pub fn of(claim: &Claim) -> ClaimKey {
        ClaimKey {
            surface: claim.surface,
            quest_id: claim.quest_id,
            field: claim.field,
            target: claim.target,
        }
    }
}

pub fn provenance_of(environment: PageEnvironment, key: &ClaimKey, era_claim_keys: &FxHashSet<ClaimKey>) -> Provenance {
    // An unknown environment cannot be shown to be Forever data, so it gets the inherited trust level.
    if environment != PageEnvironment::Forever {
        Provenance::EraPage
    } else if era_claim_keys.contains(key) {
        Provenance::ForeverEraEdge
    } else {
        Provenance::ForeverNewEdge
    }
}

/// Score for claim kinds that were never measured (no rows in [`measured`]).
const UNMEASURED: f64 = 0.1;

pub fn base_score(surface: Surface, field: RelationField, shape: ClaimShape, provenance: Provenance) -> f64 {
    measured(surface, field, shape).map_or(UNMEASURED, |by_provenance| by_provenance[provenance as usize])
}

/// Scores by provenance, in [`Provenance`] order: forever-new-edge, forever-era-edge, era-page.
///
/// Comments give TP/judged per provenance as measured on 2026-10-10 (forever-new-edge on
/// authoredForever, the others on inheritedClassic): agreement with current Questie data, which
/// can itself be wrong or behind Forever. Panels only exist on Era pages, so their other
/// provenances are unmeasured guesses.
fn measured(surface: Surface, field: RelationField, shape: ClaimShape) -> Option<[f64; 3]> {
    use ClaimShape::*;
    use RelationField::*;
    use Surface::*;
    Some(match (surface, field, shape) {
        // 42/47, 863/875, 1/1
        (Series, PreQuestSingle, Single) => [0.85, 0.95, 0.8],
        // 2/2, 88/98, 0/42 (Era pages list Era-only duplicates of a step)
        (Series, PreQuestSingle, Variants) => [0.7, 0.85, 0.1],
        // 2/4, 16/17, none
        (Series, PreQuestSingle, Group) => [0.5, 0.8, 0.4],
        // 6/14, 253/428, 0/1: the rest are breadcrumbs (below)
        (Series, PreQuestSingle, LeadIn) => [0.4, 0.55, 0.3],
        // 39/48, 934/935, 1/1
        (Series, NextQuestInChain, Single) => [0.8, 0.95, 0.8],
        // 5/6, 418/425, 0/1
        (Series, NextQuestInChain, LeadIn) => [0.75, 0.95, 0.6],
        // 2/6, 161/425, 0/1
        (Series, BreadcrumbForQuestId, LeadIn) => [0.25, 0.35, 0.2],
        // 2/10, 160/428, 0/1
        (Series, Breadcrumbs, LeadIn) => [0.2, 0.35, 0.2],
        // unjudged: no Toxic Soil quest is in the ground truth yet
        (Storyline, PreQuestSingle, Single) => [0.5, 0.5, 0.5],
        (Storyline, PreQuestSingle, Variants) => [0.5, 0.5, 0.5],
        // era-page 97/101
        (Requires, PreQuestSingle, Single) => [0.85, 0.9, 0.9],
        // era-page 8/8 (one page)
        (Requires, PreQuestSingle, Variants) => [0.7, 0.8, 0.8],
        // never observed
        (Requires, PreQuestGroup, Group) => [0.4, 0.5, 0.5],
        // era-page 52/329: repeatable turn-in groups that list themselves
        (RequiresAny, PreQuestSingle, WithSelf) => [0.15, 0.15, 0.15],
        // era-page 3/3
        (RequiresInProgress, ParentQuest, Single) => [0.7, 0.75, 0.75],
        // era-page 37/41
        (Unlocks, PreQuestSingle, Single) => [0.8, 0.85, 0.85],
        // era-page 21/99
        (Unlocks, PreQuestSingle, Variants) => [0.2, 0.2, 0.2],
        // era-page 0/147
        (Unlocks, PreQuestSingle, WithSelf) => [0.05, 0.05, 0.05],
        // era-page 36/229
        (Disables, ExclusiveTo, Variants) => [0.15, 0.15, 0.15],
        // era-page 0/279
        (Disables, ExclusiveTo, Group) => [0.05, 0.05, 0.05],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_separates_forever_only_claims_from_inherited_ones() {
        let key = |quest_id| ClaimKey {
            surface: Surface::Series,
            quest_id,
            field: RelationField::PreQuestSingle,
            target: 1,
        };
        let era = FxHashSet::from_iter([key(2)]);
        assert_eq!(provenance_of(PageEnvironment::Forever, &key(2), &era), Provenance::ForeverEraEdge);
        assert_eq!(provenance_of(PageEnvironment::Forever, &key(3), &era), Provenance::ForeverNewEdge);
        assert_eq!(provenance_of(PageEnvironment::Era, &key(3), &era), Provenance::EraPage);
        assert_eq!(provenance_of(PageEnvironment::Unknown, &key(3), &era), Provenance::EraPage);
    }

    #[test]
    fn scores_are_read_by_provenance_and_unmeasured_kinds_get_the_floor() {
        use Provenance::*;
        let variants = |provenance| base_score(Surface::Series, RelationField::PreQuestSingle, ClaimShape::Variants, provenance);
        assert_eq!([variants(ForeverNewEdge), variants(ForeverEraEdge), variants(EraPage)], [0.7, 0.85, 0.1]);
        assert_eq!(
            base_score(Surface::RequiresAny, RelationField::PreQuestSingle, ClaimShape::Single, EraPage),
            UNMEASURED
        );
    }
}
