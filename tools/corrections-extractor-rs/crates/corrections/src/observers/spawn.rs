//! Spawn observations, shared by NPC and object spawns and by quest turn-in POIs (which join
//! their finisher's spawns at finalize).

/// Where a spawn position came from. Merges rank positions by it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SpawnToken {
    Target,
    Npc,
    QuestNpc,
    /// A quest turn-in POI attached to the quest's single finisher.
    QuestPoi,
}

impl SpawnToken {
    /// The unit token an encounter was seen through.
    pub fn from_unit_token(token: &str) -> Option<SpawnToken> {
        match token {
            "target" => Some(SpawnToken::Target),
            "npc" => Some(SpawnToken::Npc),
            "questnpc" => Some(SpawnToken::QuestNpc),
            "questpoi" => Some(SpawnToken::QuestPoi),
            _ => None,
        }
    }
}

/// One spawn point seen in a zone (TS `SpawnObservationValue` plus the tagged `token`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpawnObservation {
    /// AreaTable areaID.
    pub zone_id: f64,
    /// Questie map coordinates, 0-100.
    pub x: f64,
    pub y: f64,
    pub token: SpawnToken,
}
