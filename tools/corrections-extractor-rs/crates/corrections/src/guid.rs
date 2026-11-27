//! WoW GUIDs as they appear in sessions, e.g. "Creature-0-5208-0-7-823-000031" or
//! "GameObject-0-5208-0-7-2843-0000399". The entity id is always the second-to-last
//! "-"-separated segment and the spawn UID the last. Ports `extract/guid.ts`.
//!
//! This only routes a GUID to an entity kind; the addon already keeps player GUIDs out of
//! traces (Modules/Privacy.lua).

use crate::js;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GuidKind {
    Npc,
    Object,
    Player,
    Item,
    Unknown,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ParsedGuid<'g> {
    pub kind: GuidKind,
    /// npcID or objectID; only for npc and object GUIDs.
    pub id: Option<f64>,
    pub spawn_uid: Option<&'g str>,
}

pub fn parse_guid(guid: Option<&str>) -> ParsedGuid<'_> {
    let unknown = ParsedGuid {
        kind: GuidKind::Unknown,
        id: None,
        spawn_uid: None,
    };
    let Some(guid) = guid.filter(|g| !g.is_empty()) else { return unknown };
    let parts: Vec<&str> = guid.split('-').collect();
    let kind = match parts[0] {
        "Creature" | "Pet" | "Vehicle" => GuidKind::Npc,
        "GameObject" => GuidKind::Object,
        "Player" => GuidKind::Player,
        "Item" => GuidKind::Item,
        _ => GuidKind::Unknown,
    };
    if matches!(kind, GuidKind::Npc | GuidKind::Object) && parts.len() >= 3 {
        let id = js::number(parts[parts.len() - 2]);
        return ParsedGuid {
            kind,
            id: id.is_finite().then_some(id),
            spawn_uid: Some(parts[parts.len() - 1]),
        };
    }
    ParsedGuid {
        kind,
        id: None,
        spawn_uid: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creature_and_game_object_guids_carry_their_entity_id() {
        let npc = parse_guid(Some("Creature-0-5208-0-7-823-000031"));
        assert_eq!((npc.kind, npc.id, npc.spawn_uid), (GuidKind::Npc, Some(823.0), Some("000031")));
        let object = parse_guid(Some("GameObject-0-5208-0-7-2843-0000399"));
        assert_eq!((object.kind, object.id), (GuidKind::Object, Some(2843.0)));
    }

    #[test]
    fn players_and_garbage_have_no_entity_id() {
        assert_eq!(parse_guid(Some("Player-1234-0053656F")).id, None);
        assert_eq!(parse_guid(Some("")).kind, GuidKind::Unknown);
        assert_eq!(parse_guid(None).kind, GuidKind::Unknown);
    }
}
