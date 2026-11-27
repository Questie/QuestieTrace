//! Edge features -> score: the measured precision of the edge's bucket, Laplace-smoothed
//! ((tp + 1) / (n + 2)) so small buckets stay below certainty. Buckets are pooled across the two
//! truth tiers where they agree, and split by tier where they do not: Forever's authored data
//! lists co-completed copies, and inherited Classic data predates the prerequisites Forever added
//! to old quests. Counts below are tp/n from the 2026-10-11 run (1,843 characters); re-measure
//! with the TypeScript signal's `--dump` (every edge with its features) when the rules or the
//! data change much.

use super::infer::{EdgeFeatures, EdgeShape, EdgeTier, PrerequisiteField};

/// Supporting characters from which an edge counts as well sampled.
const MANY_CHARACTERS: u32 = 10;

pub fn score_edge(f: &EdgeFeatures) -> f64 {
    let level_unlocked = f.level_unlocked_share > 0.0;
    let many = f.support >= MANY_CHARACTERS;

    if let PrerequisiteField::Group { all_of_confirmed } = f.field {
        // Unconfirmed (play order only): 3/13 pooled, Classic 0/10 and Forever 3/3 from one quest.
        if !all_of_confirmed {
            return 0.27;
        }
        // Confirmed: Forever 15/15; Classic 5/9, the misses being Forever quests newly required by old ones.
        return if f.forever { 0.94 } else { 0.55 };
    }

    match (f.shape, f.tier) {
        // Faction-split OR members: 16/16 (Forever only).
        (EdgeShape::Or, _) => 0.94,
        // Rare since copies cover most races: 1 emitted, unjudged.
        (EdgeShape::Variant, _) => 0.5,
        // Questie never lists Forever's copies on Classic quests: 2/21.
        (EdgeShape::Copy, _) if !f.forever => 0.13,
        // Copies of Forever quests' prerequisites: 404/412 (0/2 when levelling explains the offer).
        (EdgeShape::Copy, EdgeTier::Primary) => {
            if level_unlocked {
                0.25
            } else {
                0.98
            }
        }
        // Secondary, own or copy: 30/38 from 10+ characters, 3/10 from 5-9.
        (_, EdgeTier::Secondary) => {
            if many {
                0.78
            } else {
                0.33
            }
        }
        // Primary: 10+ characters 574/581 (51/53 with a level-up just before the first offer),
        // 2-9 characters 161/167 (20/22).
        (EdgeShape::Direct, EdgeTier::Primary) => match (many, level_unlocked) {
            (true, true) => 0.95,
            (true, false) => 0.98,
            (false, true) => 0.88,
            (false, false) => 0.96,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primary() -> EdgeFeatures {
        EdgeFeatures {
            field: PrerequisiteField::Single,
            forever: true,
            tier: EdgeTier::Primary,
            shape: EdgeShape::Direct,
            characters: 40,
            support: 40,
            contradict: 0,
            link: true,
            pending: 20,
            background: 0,
            fresh: 30,
            immediate: 30,
            latest: 30,
            level_unlocked_share: 0.0,
            remaining: 1,
        }
    }

    #[test]
    fn scores_edges_from_few_characters_below_the_same_evidence_from_many() {
        let few = EdgeFeatures {
            characters: 2,
            support: 2,
            ..primary()
        };
        assert!(score_edge(&few) < score_edge(&primary()));
        let secondary = EdgeFeatures {
            tier: EdgeTier::Secondary,
            link: false,
            ..primary()
        };
        let few_secondary = EdgeFeatures {
            characters: 6,
            support: 6,
            ..secondary.clone()
        };
        assert!(score_edge(&few_secondary) < score_edge(&secondary));
    }

    #[test]
    fn scores_a_group_backed_only_by_play_order_far_below_a_confirmed_one() {
        let group = |all_of_confirmed| EdgeFeatures {
            field: PrerequisiteField::Group { all_of_confirmed },
            remaining: 2,
            ..primary()
        };
        assert!(score_edge(&group(false)) < 0.5);
        assert!(score_edge(&group(true)) > 0.9);
    }
}
