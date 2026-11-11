//! Dense indexes over the characters and quests the signal reasons about, so the pairwise passes
//! count over plain vectors instead of maps.

use super::history::CharacterHistory;
use crate::eligibility::{class_allowed, race_allowed};
use crate::model::{CatalogQuest, QuestCatalog, QuestId};
use rustc_hash::{FxHashMap, FxHashSet};

/// Characters sharing race and class: every quest's race/class gate treats them alike.
pub struct Cell {
    /// True where the cell's race and class may take the quest at that index: the catalog's masks,
    /// widened by what characters of the cell actually completed. Catalog masks are sometimes too
    /// narrow (Forever's 91743 excludes humans, yet every human on that path completed it).
    pub allows: Vec<bool>,
}

pub struct Character<'e> {
    pub history: CharacterHistory<'e>,
    /// Index into [`Population::cells`].
    pub cell: usize,
    /// Quest indexes this character completed, ascending.
    pub done: Vec<usize>,
}

pub struct Population<'e> {
    /// Catalog quests the signal reasons about, by ascending id; a quest index points in here.
    pub quests: Vec<&'e CatalogQuest>,
    index_of: FxHashMap<QuestId, usize>,
    /// In the order their first character appears.
    pub cells: Vec<Cell>,
    /// In history order; a character index points in here.
    pub characters: Vec<Character<'e>>,
    /// Per quest index, the characters that took it (held, accepted or completed), ascending.
    pub takers: Vec<Vec<usize>>,
    /// Per quest index, the characters that completed it, ascending.
    pub completers: Vec<Vec<usize>>,
}

impl<'e> Population<'e> {
    pub fn new(histories: Vec<CharacterHistory<'e>>, catalog: &'e QuestCatalog, ignore: &FxHashSet<QuestId>) -> Self {
        let quests: Vec<&CatalogQuest> = catalog.quests.values().filter(|quest| !ignore.contains(&quest.id)).collect();
        let index_of: FxHashMap<QuestId, usize> = quests.iter().enumerate().map(|(index, quest)| (quest.id, index)).collect();

        let mut cells: Vec<Cell> = Vec::new();
        let mut cell_of: FxHashMap<(Option<i64>, Option<i64>), usize> = FxHashMap::default();
        let characters: Vec<Character> = histories
            .into_iter()
            .map(|history| {
                let (race_id, class_id) = (history.player.race_id, history.player.class_id);
                let cell = *cell_of.entry((race_id, class_id)).or_insert_with(|| {
                    let allows = quests
                        .iter()
                        .map(|quest| race_allowed(quest.required_races, race_id) && class_allowed(quest.required_classes, class_id))
                        .collect();
                    cells.push(Cell { allows });
                    cells.len() - 1
                });
                let mut done: Vec<usize> = history.done.keys().filter_map(|quest_id| index_of.get(quest_id).copied()).collect();
                done.sort_unstable();
                Character { history, cell, done }
            })
            .collect();

        let mut takers = vec![Vec::new(); quests.len()];
        let mut completers = vec![Vec::new(); quests.len()];
        for (character_index, character) in characters.iter().enumerate() {
            for quest_id in character.history.taken.keys() {
                if let Some(&index) = index_of.get(quest_id) {
                    takers[index].push(character_index);
                }
            }
            for &index in &character.done {
                completers[index].push(character_index);
                cells[character.cell].allows[index] = true;
            }
        }

        Population {
            quests,
            index_of,
            cells,
            characters,
            takers,
            completers,
        }
    }

    pub fn index_of(&self, quest_id: QuestId) -> Option<usize> {
        self.index_of.get(&quest_id).copied()
    }

    /// Race and class allow the character the quest at `quest_index`.
    pub fn allows(&self, character: &Character, quest_index: usize) -> bool {
        self.cells[character.cell].allows[quest_index]
    }

    /// Race/class allow the quest and the character was seen at or above its required level.
    pub fn could_take(&self, character: &Character, quest_index: usize) -> bool {
        if !self.allows(character, quest_index) {
            return false;
        }
        match (self.quests[quest_index].required_level, character.history.max_level) {
            (Some(required), Some(level)) => level >= required,
            _ => true,
        }
    }
}
