//! Builders for small episodes and catalogs in tests: sensible defaults, override only what the
//! test is about.

use crate::model::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_EPISODE: AtomicUsize = AtomicUsize::new(0);

/// A level 10 human warrior on a Forever build, an hour long, with nothing completed.
pub fn make_episode() -> QuestEpisode {
    let n = NEXT_EPISODE.fetch_add(1, Ordering::Relaxed) + 1;
    QuestEpisode {
        key: format!("test-{n}"),
        contributor_id: "contributor".into(),
        character_key: format!("character-{n}"),
        submission_ids: vec![format!("submission-{n}")],
        session_name: None,
        session_order: 0,
        interface_version: Some(16001),
        locale: Some("enUS".into()),
        player: PlayerContext {
            race_id: Some(1),
            class_id: Some(1),
            faction: Some(Faction::Alliance),
        },
        duration: 3600.0,
        levels: vec![TimedValue { t: 0.0, v: 10 }],
        completed: Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: vec![],
            changes: vec![],
        }),
        quest_log: vec![TimedValue { t: 0.0, v: vec![] }],
        quest_events: vec![],
        offers: vec![],
        quest_info: None,
    }
}

/// A character id for `episode` in tests that feed tallies directly: the same for every episode
/// of a character (a hash of its key; signals intern keys with `CharacterIds` instead).
pub fn character_id(episode: &QuestEpisode) -> crate::signals::CharacterId {
    use std::hash::Hasher;
    let mut hasher = rustc_hash::FxHasher::default();
    hasher.write(episode.character_key.as_bytes());
    crate::signals::CharacterId::new(hasher.finish() as u32)
}

pub fn npc(id: i64) -> GiverRef {
    GiverRef { kind: GiverKind::Npc, id }
}

pub fn event(t: f64, kind: QuestEventKind, quest_id: QuestId, giver: Option<GiverRef>) -> QuestEvent {
    QuestEvent { t, kind, quest_id, giver }
}

/// A complete gossip offer list.
pub fn gossip(t: f64, giver: GiverRef, available: &[QuestId], active: &[QuestId]) -> OfferSnapshot {
    OfferSnapshot {
        t,
        source: OfferSource::Gossip,
        giver: Some(giver),
        available: available
            .iter()
            .map(|&id| OfferedQuest {
                id,
                trivial: None,
                frequency: None,
                repeatable: None,
            })
            .collect(),
        unresolved_titles: None,
        active: Some(active.to_vec()),
        list_complete: true,
    }
}

pub fn catalog_quest(id: QuestId) -> CatalogQuest {
    CatalogQuest { id, ..CatalogQuest::default() }
}

/// A catalog holding `quests`.
pub fn catalog(quests: impl IntoIterator<Item = CatalogQuest>) -> QuestCatalog {
    QuestCatalog {
        quests: quests.into_iter().map(|quest| (quest.id, quest)).collect(),
        ..QuestCatalog::default()
    }
}
