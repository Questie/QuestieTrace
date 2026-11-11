//! Could this character have taken this quest at all, ignoring relationships? Signals use this
//! to gate negative evidence: "X was not offered" only means something if the character was
//! otherwise eligible for X.

use crate::model::{CatalogQuest, PlayerContext, QuestId};
use crate::race_masks::{class_bit, race_bit};

/// Highest quest id in Classic Era data. Higher ids are Forever-new content.
pub const MAX_CLASSIC_QUEST_ID: QuestId = 9665;

pub fn is_forever_quest_id(quest_id: QuestId) -> bool {
    quest_id > MAX_CLASSIC_QUEST_ID
}

/// Forever builds report interface versions 16xxx; old traces have no version and are kept.
pub fn is_forever_interface(interface_version: Option<i64>) -> bool {
    interface_version.is_none_or(|version| version.to_string().starts_with("16"))
}

/// An absent or 0 mask means everyone; unknown race ids are allowed.
pub fn race_allowed(mask: Option<u64>, race_id: Option<i64>) -> bool {
    match (mask, race_id.and_then(race_bit)) {
        (Some(mask), Some(bit)) if mask != 0 => mask & bit != 0,
        _ => true,
    }
}

/// An absent or 0 mask means everyone; ids below 1 are allowed.
pub fn class_allowed(mask: Option<u64>, class_id: Option<i64>) -> bool {
    match (mask, class_id.and_then(class_bit)) {
        (Some(mask), Some(bit)) if mask != 0 => mask & bit != 0,
        _ => true,
    }
}

/// Static eligibility only: race, class and level. `level_slack` is how many levels below the
/// catalog's requiredLevel to tolerate (catalog levels are not always exact; callers checking
/// negative evidence may pass a negative value to require a margin instead). Skills, reputation
/// and spells are not tracked per character: see [`has_untracked_requirements`].
pub fn is_statically_eligible(quest: &CatalogQuest, player: &PlayerContext, level: Option<i64>, level_slack: i64) -> bool {
    if !race_allowed(quest.required_races, player.race_id) || !class_allowed(quest.required_classes, player.class_id) {
        return false;
    }
    match (quest.required_level, level) {
        (Some(required), Some(level)) => level + level_slack >= required,
        _ => true,
    }
}

/// Requirements a trace cannot check: skill, reputation, spell knowledge.
pub fn has_untracked_requirements(quest: &CatalogQuest) -> bool {
    quest.required_skill.is_some() || quest.required_min_rep.is_some() || quest.required_max_rep.is_some() || quest.required_spell.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::catalog_quest;

    fn human_warrior() -> PlayerContext {
        PlayerContext {
            race_id: Some(1),
            class_id: Some(1),
            faction: None,
        }
    }

    #[test]
    fn race_and_class_masks_gate_and_zero_means_everyone() {
        let horde_only = CatalogQuest {
            required_races: Some(2 + 16 + 32 + 128),
            ..catalog_quest(100)
        };
        assert!(!is_statically_eligible(&horde_only, &human_warrior(), Some(10), 0));
        let anyone = CatalogQuest {
            required_races: Some(0),
            required_classes: Some(0),
            ..catalog_quest(100)
        };
        assert!(is_statically_eligible(&anyone, &human_warrior(), Some(10), 0));
        let mages = CatalogQuest {
            required_classes: Some(128),
            ..catalog_quest(100)
        };
        assert!(!is_statically_eligible(&mages, &human_warrior(), Some(10), 0));
    }

    #[test]
    fn skyborne_bits_above_32_are_checked() {
        let skyborne_alliance = CatalogQuest {
            required_races: Some(1 << 32),
            ..catalog_quest(100)
        };
        let skyborne = PlayerContext {
            race_id: Some(95),
            ..human_warrior()
        };
        assert!(is_statically_eligible(&skyborne_alliance, &skyborne, None, 0));
        assert!(!is_statically_eligible(&skyborne_alliance, &human_warrior(), None, 0));
    }

    #[test]
    fn level_slack_moves_the_level_requirement() {
        let level_ten = CatalogQuest {
            required_level: Some(10),
            ..catalog_quest(100)
        };
        assert!(!is_statically_eligible(&level_ten, &human_warrior(), Some(9), 0));
        assert!(is_statically_eligible(&level_ten, &human_warrior(), Some(9), 1));
        assert!(!is_statically_eligible(&level_ten, &human_warrior(), Some(10), -1));
        assert!(is_statically_eligible(&level_ten, &human_warrior(), None, 0), "unknown level does not gate");
    }

    #[test]
    fn forever_interfaces_start_with_16_and_missing_versions_count() {
        assert!(is_forever_interface(Some(16001)));
        assert!(is_forever_interface(None));
        assert!(!is_forever_interface(Some(11507)));
    }
}
