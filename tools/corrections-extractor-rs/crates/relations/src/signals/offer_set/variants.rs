//! Faction/race/class variants: same-name copies of a quest that no single character can both
//! take. Forever-new prerequisite lists with several entries are almost always these ("the
//! Alliance or the Horde copy"), so they are how an OR prerequisite is recognised.
//!
//! An audience is a bitmask in Questie's requiredRaces / requiredClasses bit space, limited to
//! the races and classes some catalog quest explicitly allows (the playable ones), so two
//! audiences overlap exactly when their masks share a bit.

use crate::model::{QuestCatalog, QuestId};
use crate::race_masks::{class_bit, race_bit};
use rustc_hash::{FxHashMap, FxHashSet};

const MAX_CLASS_ID: i64 = 16;

/// Every race bit Questie masks know: [`race_bit`] over all race ids (Blizzard race ids fit in
/// a byte).
fn known_race_bits() -> u64 {
    (0..=255).filter_map(race_bit).fold(0, |bits, bit| bits | bit)
}

fn known_class_bits() -> u64 {
    (1..=MAX_CLASS_ID).filter_map(class_bit).fold(0, |bits, bit| bits | bit)
}

/// The bits of `known` some quest's mask explicitly allows: the playable races/classes, not every
/// known id. Every known bit when no quest restricts at all (otherwise every audience would be
/// empty).
fn bits_in_use(masks: impl Iterator<Item = Option<u64>>, known: u64) -> u64 {
    let used = masks.flatten().fold(0, |bits, mask| bits | (mask & known));
    if used == 0 {
        known
    } else {
        used
    }
}

/// The part of `universe` a quest's mask allows; an absent or 0 mask allows all of it.
fn audience(mask: Option<u64>, universe: u64) -> u64 {
    match mask {
        Some(mask) if mask != 0 => mask & universe,
        _ => universe,
    }
}

pub struct VariantIndex<'a> {
    catalog: &'a QuestCatalog,
    /// Quest ids per non-empty name, ascending.
    by_name: FxHashMap<&'a str, Vec<QuestId>>,
    race_universe: u64,
    class_universe: u64,
    /// Quests with at least one variant.
    with_variants: FxHashSet<QuestId>,
}

impl<'a> VariantIndex<'a> {
    pub fn new(catalog: &'a QuestCatalog) -> Self {
        let quests = || catalog.quests.values();
        let mut by_name: FxHashMap<&str, Vec<QuestId>> = FxHashMap::default();
        for quest in quests() {
            if let Some(name) = quest.name.as_deref().filter(|name| !name.is_empty()) {
                by_name.entry(name).or_default().push(quest.id);
            }
        }
        let mut index = VariantIndex {
            catalog,
            by_name,
            race_universe: bits_in_use(quests().map(|quest| quest.required_races), known_race_bits()),
            class_universe: bits_in_use(quests().map(|quest| quest.required_classes), known_class_bits()),
            with_variants: FxHashSet::default(),
        };
        index.with_variants = quests().map(|quest| quest.id).filter(|&id| !index.variants_of(id).is_empty()).collect();
        index
    }

    pub fn has_variants(&self, quest_id: QuestId) -> bool {
        self.with_variants.contains(&quest_id)
    }

    /// The quest's name, when it has a non-empty one.
    pub fn name_of(&self, quest_id: QuestId) -> Option<&'a str> {
        self.catalog.quest(quest_id)?.name.as_deref().filter(|name| !name.is_empty())
    }

    fn races_of(&self, quest_id: QuestId) -> u64 {
        audience(self.catalog.quest(quest_id).and_then(|quest| quest.required_races), self.race_universe)
    }

    fn classes_of(&self, quest_id: QuestId) -> u64 {
        audience(self.catalog.quest(quest_id).and_then(|quest| quest.required_classes), self.class_universe)
    }

    /// True when no character can take both quests (disjoint races or disjoint classes).
    pub fn exclusive_audiences(&self, a: QuestId, b: QuestId) -> bool {
        self.races_of(a) & self.races_of(b) == 0 || self.classes_of(a) & self.classes_of(b) == 0
    }

    /// Same-name quests whose audience does not overlap `quest_id`'s, ascending.
    pub fn variants_of(&self, quest_id: QuestId) -> Vec<QuestId> {
        let Some(name) = self.name_of(quest_id) else {
            return Vec::new();
        };
        self.by_name[name]
            .iter()
            .copied()
            .filter(|&other| other != quest_id && self.exclusive_audiences(quest_id, other))
            .collect()
    }

    /// True when some race allowed to take `quest_id` is allowed by none of `prerequisites`
    /// (class gaps are ignored: class quests rarely have class-specific prerequisites for other
    /// classes). Then a preQuestSingle of exactly these quests would hide `quest_id` from that race.
    pub fn leaves_races_uncovered(&self, quest_id: QuestId, prerequisites: &[QuestId]) -> bool {
        let covered = prerequisites.iter().fold(0, |races, &prerequisite| races | self.races_of(prerequisite));
        self.races_of(quest_id) & !covered != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CatalogQuest;
    use crate::test_support::{catalog, catalog_quest};

    #[test]
    fn an_unrestricted_quest_is_open_to_the_races_some_quest_allows_not_every_known_race() {
        // No quest here allows a Horde race, so the unrestricted quest's audience is the Alliance
        // races, and the Alliance-only quest covers all of it.
        let alliance_only = CatalogQuest {
            name: Some("Report In".into()),
            required_races: Some(4_294_967_373),
            ..catalog_quest(1)
        };
        let catalog = catalog([alliance_only, catalog_quest(2)]);
        let variants = VariantIndex::new(&catalog);
        assert!(!variants.leaves_races_uncovered(2, &[1]));
        assert!(!variants.exclusive_audiences(1, 2));
    }
}
