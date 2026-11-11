//! Who takes the blame for a quest's blocked moments: EM over a mixture of causes.
//!
//! Each blocked moment has one cause: one of its suspects, or something unknown (an unmodelled
//! gate, or one of the many unlikely suspects not kept individually). A suspect's chance to be the
//! cause is its prior weight (locality.rs) times its propensity, and propensities are re-estimated
//! from the blame each cause took over all characters. The real blocker explains every character's
//! block, so it takes the blame from proxies that only explain some of them, such as quests that
//! players usually do right after the real blocker.

use super::blocked::{BlockedMoment, BlockedQuest, Explanation};
use super::offers::Character;
use super::params::AbsenceParams;
use crate::model::Evidence;
use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap};

#[derive(Clone, Debug)]
pub struct Share {
    /// Probability that this suspect caused the character's blocked moment.
    pub share: f64,
    /// The character had been offered the quest before (see BlockedMoment::transition).
    pub transition: bool,
    pub evidence: Evidence,
}

/// Character -> the character's most convincing moment for one explanation; characters in the
/// order of their first moment naming it.
pub type SharesByCharacter<'a> = IndexMap<Character<'a>, Share, FxBuildHasher>;

/// Explanation -> its shares, explanations in the order the moments first name them.
pub type Explanations<'a> = IndexMap<Explanation, SharesByCharacter<'a>, FxBuildHasher>;

/// How readily each cause blocks this quest, re-estimated every EM iteration.
struct Propensity {
    suspects: FxHashMap<Explanation, f64>,
    unknown: f64,
}

pub fn attribute<'a>(blocked: &BlockedQuest<'a>, params: &AbsenceParams) -> Explanations<'a> {
    let moments = &blocked.moments;
    // Every character counts once, however many moments they contributed.
    let mut moments_per_character: FxHashMap<Character, usize> = FxHashMap::default();
    for moment in moments {
        *moments_per_character.entry(moment.character).or_default() += 1;
    }

    let mut propensity = Propensity {
        suspects: moments
            .iter()
            .flat_map(|moment| &moment.suspects)
            .map(|suspect| (suspect.explanation, 1.0))
            .collect(),
        unknown: 1.0,
    };
    for _ in 0..params.em_iterations {
        let mut blame: FxHashMap<Explanation, f64> = FxHashMap::default();
        let mut unknown_blame = 0.0;
        let mut total_blame = 0.0;
        for moment in moments {
            let weight = 1.0 / moments_per_character[moment.character] as f64;
            let shares = responsibilities(moment, &propensity, params.unknown_weight);
            let mut explained = 0.0;
            for (suspect, share) in moment.suspects.iter().zip(shares) {
                *blame.entry(suspect.explanation).or_insert(0.0) += weight * share;
                explained += share;
            }
            unknown_blame += weight * (1.0 - explained);
            total_blame += weight;
        }
        // A small floor keeps every suspect in play; the data decides, not the starting point.
        for (explanation, value) in &mut propensity.suspects {
            *value = (blame.get(explanation).copied().unwrap_or(0.0) / total_blame).max(1e-6);
        }
        propensity.unknown = (unknown_blame / total_blame).max(1e-6);
    }

    let mut result = Explanations::default();
    for moment in moments {
        let shares = responsibilities(moment, &propensity, params.unknown_weight);
        let others = moment.suspects.len().saturating_sub(1);
        for (suspect, share) in moment.suspects.iter().zip(shares) {
            let by_character = result.entry(suspect.explanation).or_default();
            if by_character.get(moment.character).is_some_and(|best| best.share >= share) {
                continue;
            }
            let mut text = format!("{}; {}", moment.what, suspect.detail);
            if others > 0 {
                text += &format!(" ({others} other suspects)");
            }
            let evidence = Evidence {
                reference: moment.reference.to_owned(),
                t: Some(moment.t),
                text,
            };
            by_character.insert(
                moment.character,
                Share {
                    share,
                    transition: moment.transition,
                    evidence,
                },
            );
        }
    }
    result
}

/// Per suspect, its probability of being the moment's cause under the current propensities.
fn responsibilities(moment: &BlockedMoment, propensity: &Propensity, unknown_weight: f64) -> Vec<f64> {
    let raw: Vec<f64> = moment
        .suspects
        .iter()
        .map(|suspect| suspect.weight * propensity.suspects[&suspect.explanation])
        .collect();
    let unknown = (unknown_weight + moment.residual_weight) * propensity.unknown;
    let total = raw.iter().fold(unknown, |sum, value| sum + value);
    raw.into_iter().map(|value| value / total).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::absence::blocked::{BlockKind, Suspect};

    fn moment<'a>(character: &'a str, suspects: &[Explanation], residual_weight: f64) -> BlockedMoment<'a> {
        BlockedMoment {
            character,
            reference: character,
            t: 1.0,
            what: "npc:1 list at 1.0s lacked 100".into(),
            transition: false,
            suspects: suspects
                .iter()
                .map(|&explanation| Suspect {
                    explanation,
                    weight: 1.0,
                    detail: String::new(),
                })
                .collect(),
            residual_weight,
        }
    }

    fn blocked(moments: Vec<BlockedMoment>) -> BlockedQuest {
        BlockedQuest {
            moments,
            ..BlockedQuest::default()
        }
    }

    fn completed(quest: i64) -> Explanation {
        Explanation {
            kind: BlockKind::Completed,
            quest,
        }
    }

    #[test]
    fn gives_the_blame_to_the_suspect_that_explains_every_character_not_to_its_proxies() {
        // completed 7 is the real blocker; 8 and 9 are quests players usually do right after it.
        let quest = blocked(vec![
            moment("a", &[completed(7), completed(8)], 0.0),
            moment("b", &[completed(7), completed(9)], 0.0),
            moment("c", &[completed(7)], 0.0),
        ]);
        let shares = attribute(&quest, &AbsenceParams::default());
        assert!(shares[&completed(7)]["a"].share > 0.85);
        assert!(shares[&completed(8)]["a"].share < 0.1);
    }

    #[test]
    fn leaves_a_lone_suspect_weak_when_most_blocks_of_the_quest_have_no_plausible_suspect() {
        // Characters b and c only had unrelated suspects (lumped into the residual): something we
        // cannot see blocks this quest, so a's single related suspect is probably that too.
        let in_log = Explanation {
            kind: BlockKind::InLog,
            quest: 5,
        };
        let quest = blocked(vec![moment("a", &[in_log], 3.0), moment("b", &[], 3.0), moment("c", &[], 3.0)]);
        assert!(attribute(&quest, &AbsenceParams::default())[&in_log]["a"].share < 0.3);
    }
}
