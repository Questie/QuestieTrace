//! Same-name quest variants: Forever often ships one copy of a quest per faction, race or
//! profession under the same name, and a quest that needs "the" previous step accepts any copy
//! (96 of 100 multi-quest preQuestSingle lists in the authored data are such copies). The traces
//! only show the copies that observed characters took, so observed edges are extended to them.

use super::tally::Pair;
use crate::eligibility::{class_allowed, is_forever_quest_id, race_allowed};
use crate::model::{CatalogQuest, QuestCatalog, QuestEpisode, QuestId, SessionTime};
use crate::race_masks::race_bit;
use rustc_hash::{FxHashMap, FxHashSet};

/// Quest id -> the other quest ids with exactly the same name, ascending. Quests without a name
/// have none.
pub struct SameNameVariants {
    copies: FxHashMap<QuestId, Vec<QuestId>>,
}

impl SameNameVariants {
    pub fn new(catalog: &QuestCatalog) -> Self {
        let mut by_name: FxHashMap<&str, Vec<QuestId>> = FxHashMap::default();
        for quest in catalog.quests.values() {
            match quest.name.as_deref().map(str::trim) {
                Some(name) if !name.is_empty() => by_name.entry(name).or_default().push(quest.id),
                _ => {}
            }
        }
        let mut copies = FxHashMap::default();
        for ids in by_name.values().filter(|ids| ids.len() > 1) {
            for &id in ids {
                copies.insert(id, ids.iter().copied().filter(|&other| other != id).collect());
            }
        }
        SameNameVariants { copies }
    }

    pub fn of(&self, quest_id: QuestId) -> &[QuestId] {
        self.copies.get(&quest_id).map_or(&[], Vec::as_slice)
    }
}

/// Which side of an observed edge P -> X a copy stands in for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariantKind {
    /// P' -> X, another copy of the observed prerequisite.
    Prerequisite,
    /// P -> X', another copy of the observed quest.
    Quest,
    /// P' -> X', the observed edge in another copy of the whole step.
    Both,
}

impl VariantKind {
    pub fn name(self) -> &'static str {
        match self {
            VariantKind::Prerequisite => "prerequisite",
            VariantKind::Quest => "quest",
            VariantKind::Both => "both",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VariantEdge {
    pub pair: Pair,
    pub kind: VariantKind,
    /// The observed edge this one was derived from.
    pub observed: Pair,
}

/// Every race id Questie masks know: [`race_bit`] is the one list of them, and Blizzard race ids
/// fit in a byte.
fn known_race_ids() -> impl Iterator<Item = i64> {
    (0..=255).filter(|&race_id| race_bit(race_id).is_some())
}

/// Blizzard class ids, 1 (Warrior) to 13 (Evoker).
const CLASS_IDS: std::ops::RangeInclusive<i64> = 1..=13;

/// Some character could take both quests: their race and class requirements overlap.
pub fn could_share_character(a: Option<&CatalogQuest>, b: Option<&CatalogQuest>) -> bool {
    let (Some(a), Some(b)) = (a, b) else { return false };
    let races = known_race_ids().any(|race| race_allowed(a.required_races, Some(race)) && race_allowed(b.required_races, Some(race)));
    let classes = CLASS_IDS
        .into_iter()
        .any(|class| class_allowed(a.required_classes, Some(class)) && class_allowed(b.required_classes, Some(class)));
    races && classes
}

/// Edges implied by observed edges P -> X through same-name copies, one per (edge, observed edge):
/// the same edge can follow from several observed ones. A copy only stands in when a character
/// could take it together with the other side, and copies are kept on the same side of the
/// Classic/Forever id boundary as the quest they replace: Forever reworked many Classic quests
/// under their old names, and the old copy usually leads somewhere else.
pub fn variant_edges(observed: &[Pair], variants: &SameNameVariants, catalog: &QuestCatalog) -> Vec<VariantEdge> {
    let mut edges = Vec::new();
    for &edge in observed {
        // P and X sharing a name are steps of one chain; their copies are other steps, not variants.
        let name = catalog.quest(edge.prerequisite).and_then(|quest| quest.name.as_deref());
        if name.is_some() && name == catalog.quest(edge.quest).and_then(|quest| quest.name.as_deref()) {
            continue;
        }
        let prerequisites = copies_of(edge.prerequisite, variants);
        let quests = copies_of(edge.quest, variants);
        let candidates = prerequisites
            .iter()
            .map(|&prerequisite| (prerequisite, edge.quest, VariantKind::Prerequisite))
            .chain(quests.iter().map(|&quest| (edge.prerequisite, quest, VariantKind::Quest)))
            .chain(
                prerequisites
                    .iter()
                    .flat_map(|&prerequisite| quests.iter().map(move |&quest| (prerequisite, quest, VariantKind::Both))),
            );
        for (prerequisite, quest, kind) in candidates {
            if could_share_character(catalog.quest(prerequisite), catalog.quest(quest)) {
                edges.push(VariantEdge {
                    pair: Pair { prerequisite, quest },
                    kind,
                    observed: edge,
                });
            }
        }
    }
    edges
}

fn copies_of(quest_id: QuestId, variants: &SameNameVariants) -> Vec<QuestId> {
    let forever = is_forever_quest_id(quest_id);
    variants
        .of(quest_id)
        .iter()
        .copied()
        .filter(|&other| is_forever_quest_id(other) == forever)
        .collect()
}

/// Deltas adding more quests than this are catch-up reads (seen with 127-342 quests), not turn-ins.
const MAX_TURN_IN_DELTA: usize = 20;

/// The completed-quest delta that first flagged a quest: an episode and a time in it.
#[derive(Clone, Copy, PartialEq)]
struct Moment<'e> {
    episode: &'e str,
    t: SessionTime,
}

/// When each character first had the tracked quests flagged completed. Forever flags every
/// parallel copy of a quest at once when one is turned in, so in the traces true copies always
/// appear in the same completed-quest delta (1,830 of 1,830 authored same-name OR pairs), while
/// same-name steps of one chain, such as Classic's seven "The Defias Brotherhood" quests, complete
/// at separate turn-ins.
pub struct CompletionMoments<'e> {
    /// Quest id -> character key -> the first delta that flagged it. Only tracked quests.
    by_quest: FxHashMap<QuestId, FxHashMap<&'e str, Moment<'e>>>,
}

impl<'e> CompletionMoments<'e> {
    pub fn new(tracked: impl IntoIterator<Item = QuestId>) -> Self {
        CompletionMoments {
            by_quest: tracked.into_iter().map(|quest_id| (quest_id, FxHashMap::default())).collect(),
        }
    }

    /// Episodes must come in the same order every run: a character's first delta wins.
    pub fn add(&mut self, episode: &'e QuestEpisode) {
        for change in episode.completed.iter().flat_map(|completed| &completed.changes) {
            let added = change.add.as_deref().unwrap_or_default();
            if added.len() > MAX_TURN_IN_DELTA {
                continue;
            }
            for quest_id in added {
                if let Some(characters) = self.by_quest.get_mut(quest_id) {
                    characters.entry(&episode.character_key).or_insert(Moment {
                        episode: &episode.key,
                        t: change.t,
                    });
                }
            }
        }
    }

    /// Some character had both flagged in one delta, and none had them flagged separately.
    /// (The answer does not depend on the order characters are visited in.)
    pub fn always_together(&self, a: QuestId, b: QuestId) -> bool {
        let (Some(a), Some(b)) = (self.by_quest.get(&a), self.by_quest.get(&b)) else {
            return false;
        };
        let mut together = 0;
        for (character, moment) in a {
            match b.get(character) {
                Some(other) if other == moment => together += 1,
                Some(_) => return false,
                None => {}
            }
        }
        together > 0
    }
}

/// The quests a [`CompletionMoments`] needs to track for these edges.
pub fn quests_in_variant_edges(edges: &[VariantEdge]) -> FxHashSet<QuestId> {
    edges
        .iter()
        .flat_map(|edge| [edge.pair.prerequisite, edge.pair.quest, edge.observed.prerequisite, edge.observed.quest])
        .collect()
}

/// Whether every copy in the edge was seen completing together with the quest it stands in for.
pub fn is_parallel_copy(edge: &VariantEdge, moments: &CompletionMoments) -> bool {
    let prerequisite_ok = edge.kind == VariantKind::Quest || moments.always_together(edge.pair.prerequisite, edge.observed.prerequisite);
    let quest_ok = edge.kind == VariantKind::Prerequisite || moments.always_together(edge.pair.quest, edge.observed.quest);
    prerequisite_ok && quest_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CompletedChange, CompletedSource, CompletedTimeline};
    use crate::test_support::{catalog, catalog_quest, make_episode};

    const ALLIANCE: u64 = 1 + 4 + 8 + 64;
    const HORDE: u64 = 2 + 16 + 32 + 128;

    fn named(id: QuestId, name: &str, required_races: Option<u64>) -> CatalogQuest {
        CatalogQuest {
            name: Some(name.into()),
            required_races,
            ..catalog_quest(id)
        }
    }

    fn edges_for(quests: Vec<CatalogQuest>, prerequisite: QuestId, quest: QuestId) -> Vec<(VariantKind, QuestId, QuestId)> {
        let catalog = catalog(quests);
        let variants = SameNameVariants::new(&catalog);
        variant_edges(&[Pair { prerequisite, quest }], &variants, &catalog)
            .into_iter()
            .map(|edge| (edge.kind, edge.pair.prerequisite, edge.pair.quest))
            .collect()
    }

    #[test]
    fn an_observed_edge_extends_to_copies_a_character_could_take_with_the_other_side() {
        let quests = vec![
            named(10001, "Intro", Some(ALLIANCE)),
            named(10002, "Intro", Some(HORDE)),
            named(10003, "Intro", None),
            named(10010, "Next", Some(ALLIANCE)),
        ];
        // The Horde copy can never precede the Alliance-only quest.
        assert_eq!(edges_for(quests, 10001, 10010), [(VariantKind::Prerequisite, 10003, 10010)]);
    }

    #[test]
    fn classic_namesakes_of_forever_quests_and_steps_of_a_same_name_chain_are_no_copies() {
        let namesake = vec![named(10001, "Intro", None), named(500, "Intro", None), named(10010, "Next", None)];
        assert_eq!(edges_for(namesake, 10001, 10010), []);
        let chain = vec![named(10001, "Step", None), named(10002, "Step", None), named(10003, "Step", None)];
        assert_eq!(edges_for(chain, 10001, 10002), []);
    }

    const EDGE: VariantEdge = VariantEdge {
        pair: Pair { prerequisite: 2, quest: 9 },
        kind: VariantKind::Prerequisite,
        observed: Pair { prerequisite: 1, quest: 9 },
    };

    fn flagged(changes: &[(f64, Vec<QuestId>)]) -> QuestEpisode {
        let changes = changes
            .iter()
            .map(|(t, add)| CompletedChange {
                t: *t,
                add: Some(add.clone()),
                remove: None,
            })
            .collect();
        QuestEpisode {
            completed: Some(CompletedTimeline {
                source: CompletedSource::Modern,
                initial: vec![],
                changes,
            }),
            ..make_episode()
        }
    }

    #[test]
    fn copies_some_character_had_flagged_in_one_delta_and_none_separately_are_parallel() {
        let together = flagged(&[(10.0, vec![1, 2])]);
        let apart = flagged(&[(10.0, vec![1]), (50.0, vec![2])]);
        let mut moments = CompletionMoments::new([1, 2]);
        moments.add(&together);
        assert!(is_parallel_copy(&EDGE, &moments));
        moments.add(&apart);
        assert!(!is_parallel_copy(&EDGE, &moments));
    }

    #[test]
    fn bulk_catch_up_deltas_which_flag_whole_chains_at_once_are_ignored() {
        let catch_up = flagged(&[(10.0, [1, 2].into_iter().chain(100..130).collect())]);
        let mut moments = CompletionMoments::new([1, 2]);
        moments.add(&catch_up);
        assert!(!is_parallel_copy(&EDGE, &moments));
    }
}
