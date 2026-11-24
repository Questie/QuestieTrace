//! Which faction takes each quest, as recorded. Turning in a quest completes its versions for the
//! other faction as well, so the traces see both done and claim both as prerequisites of the next
//! step. QuestieDB lists the versions a quest's own faction has access to, and no others (agreed
//! with its maintainer on 2026-11-24).
//!
//! By the recordings and not by QuestieDB's race masks, which are wrong on dozens of quests.

use crate::model::{Faction, QuestEpisode, QuestEventKind, QuestId};
use rustc_hash::{FxHashMap, FxHashSet};

/// A quest fewer characters took could still be for both factions.
const MIN_TAKERS: u32 = 3;

/// Per quest, how many characters of each faction accepted it or turned it in.
#[derive(Default)]
pub struct Takers(FxHashMap<QuestId, ByFaction>);

#[derive(Clone, Copy, Default)]
struct ByFaction {
    alliance: u32,
    horde: u32,
}

impl ByFaction {
    /// The one faction that takes the quest; None for both, or with too few characters to tell.
    fn only(self) -> Option<Faction> {
        match (self.alliance, self.horde) {
            (MIN_TAKERS.., 0) => Some(Faction::Alliance),
            (0, MIN_TAKERS..) => Some(Faction::Horde),
            _ => None,
        }
    }
}

/// The quests a character accepted or turned in, from its sessions.
pub fn quests_taken(sessions: &[&QuestEpisode]) -> FxHashSet<QuestId> {
    let events = sessions.iter().flat_map(|session| &session.quest_events);
    events
        .filter(|event| matches!(event.kind, QuestEventKind::Accepted | QuestEventKind::TurnedIn))
        .map(|event| event.quest_id)
        .collect()
}

impl Takers {
    /// Adds the quests one character took (see [`quests_taken`]).
    pub fn add(&mut self, faction: Option<Faction>, quests: FxHashSet<QuestId>) {
        for quest in quests {
            let takers = self.0.entry(quest).or_default();
            match faction {
                Some(Faction::Alliance) => takers.alliance += 1,
                Some(Faction::Horde) => takers.horde += 1,
                Some(Faction::Neutral) | None => {}
            }
        }
    }

    /// Whether one faction takes `quest` and only the other one takes `prerequisite`.
    pub fn is_for_the_other_faction(&self, quest: QuestId, prerequisite: QuestId) -> bool {
        let only = |quest: QuestId| self.0.get(&quest).copied().unwrap_or_default().only();
        match (only(quest), only(prerequisite)) {
            (Some(one), Some(other)) => one != other,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{event, make_episode};

    fn takers(counts: &[(QuestId, Faction, u32)]) -> Takers {
        let mut takers = Takers::default();
        for &(quest, faction, characters) in counts {
            for _ in 0..characters {
                takers.add(Some(faction), FxHashSet::from_iter([quest]));
            }
        }
        takers
    }

    #[test]
    fn a_version_only_the_other_faction_takes_is_for_the_other_faction() {
        use Faction::{Alliance, Horde};
        // "The Fate of Zephras" (Horde) after the Horde's and the Alliance's "Confront Lorthuna".
        let takers = takers(&[(93836, Horde, 3), (92646, Horde, 3), (93835, Alliance, 3)]);
        assert!(takers.is_for_the_other_faction(93836, 93835));
        assert!(!takers.is_for_the_other_faction(93836, 92646));
    }

    #[test]
    fn quests_both_factions_take_or_too_few_characters_took_are_for_anyone() {
        use Faction::{Alliance, Horde};
        let takers = takers(&[
            (1, Horde, 3),
            (2, Alliance, 100),
            (2, Horde, 1), // one is enough: the server let it
            (3, Alliance, 2),
            (4, Horde, 2),
            (5, Alliance, 3),
            (6, Alliance, 3),
            (6, Horde, 3),
        ]);
        assert!(!takers.is_for_the_other_faction(1, 2), "both factions take the prerequisite");
        assert!(!takers.is_for_the_other_faction(1, 6) && !takers.is_for_the_other_faction(5, 6));
        assert!(!takers.is_for_the_other_faction(6, 5), "both factions take the quest");
        assert!(!takers.is_for_the_other_faction(1, 3), "two characters took the prerequisite");
        assert!(!takers.is_for_the_other_faction(4, 5), "two characters took the quest");
        assert!(!takers.is_for_the_other_faction(1, 99), "nobody was seen taking the prerequisite");
        assert!(takers.is_for_the_other_faction(1, 5));
    }

    #[test]
    fn a_character_takes_what_it_accepts_or_turns_in_once_however_often() {
        let mut first = make_episode();
        first.quest_events = vec![
            event(1.0, QuestEventKind::Accepted, 10, None),
            event(2.0, QuestEventKind::Detail, 11, None),
            event(3.0, QuestEventKind::Abandoned, 12, None),
        ];
        let mut second = make_episode();
        second.quest_events = vec![event(1.0, QuestEventKind::TurnedIn, 10, None), event(2.0, QuestEventKind::TurnedIn, 13, None)];
        let taken = quests_taken(&[&first, &second]);
        assert_eq!(taken, FxHashSet::from_iter([10, 13]));
    }
}
