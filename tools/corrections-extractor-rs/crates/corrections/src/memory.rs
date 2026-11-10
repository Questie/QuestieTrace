//! Approximate memory the fold holds per field (`--stats`), to see what to shrink.
//!
//! Sizes are estimates: a value's own size plus the heap it owns (by capacity), plus a rough
//! per-entry overhead for the maps holding them. Good enough to rank fields, not to the byte.

use crate::aggregate::Num;
use crate::observation::Localized;
use crate::observers::item::{npc_drops::NpcDrop, object_drops::ObjectDrop};
use crate::observers::npc::friendly_to_faction::FactionReactionValue;
use crate::observers::quest::finished_by::FinishedByFinisher;
use crate::observers::quest::objectives::ObjectiveMatch;
use crate::observers::quest::quest_xp::QuestXpValue;
use crate::observers::quest::reputation_reward::ReputationTurnInValue;
use crate::observers::quest::started_by::StartedByStarter;
use crate::observers::quest::trigger_end::QuestTriggerEndValue;
use crate::observers::quest::turn_in_poi::TurnInPoiValue;
use crate::observers::spawn::SpawnObservation;
use crate::probes::Faction;

/// Heap bytes a value owns, beyond its own size.
pub trait HeapSize {
    fn heap_size(&self) -> usize {
        0
    }
}

macro_rules! no_heap {
    ($($t:ty),*) => { $(impl HeapSize for $t {})* };
}

no_heap!(
    f64,
    i64,
    Num,
    Faction,
    SpawnObservation,
    TurnInPoiValue,
    QuestXpValue,
    FactionReactionValue,
    NpcDrop,
    ObjectDrop,
    StartedByStarter,
    FinishedByFinisher
);

impl HeapSize for String {
    fn heap_size(&self) -> usize {
        self.capacity()
    }
}

impl<T: HeapSize> HeapSize for Vec<T> {
    fn heap_size(&self) -> usize {
        self.capacity() * std::mem::size_of::<T>() + self.iter().map(HeapSize::heap_size).sum::<usize>()
    }
}

impl<A: HeapSize, B: HeapSize> HeapSize for (A, B) {
    fn heap_size(&self) -> usize {
        self.0.heap_size() + self.1.heap_size()
    }
}

impl<T: HeapSize> HeapSize for Localized<T> {
    fn heap_size(&self) -> usize {
        self.locale.heap_size() + self.value.heap_size()
    }
}

impl HeapSize for ObjectiveMatch {
    fn heap_size(&self) -> usize {
        self.name.heap_size() + self.text.heap_size()
    }
}

impl HeapSize for QuestTriggerEndValue {
    fn heap_size(&self) -> usize {
        self.text.heap_size() + self.locations.heap_size()
    }
}

impl HeapSize for ReputationTurnInValue {
    fn heap_size(&self) -> usize {
        self.gains.heap_size() + self.spillover.heap_size()
    }
}

/// Rough bytes per map entry beyond the stored key and value (hash table slot and control).
pub const MAP_ENTRY_OVERHEAD: usize = 16;

/// One line of the report.
pub struct FieldMemory {
    pub field: &'static str,
    /// Observations kept, or value groups for summarized fields.
    pub rows: usize,
    pub bytes: usize,
}
