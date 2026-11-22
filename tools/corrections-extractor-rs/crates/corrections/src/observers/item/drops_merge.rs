//! The merges of item.npcDrops and item.objectDrops. Port of
//! `extract/observers/item/dropsMerge.ts`.
//!
//! Each observation carries one source id; the merge unions them per item into ids
//! ascending without duplicates (written out as a JS `number[]`). World-drop items are left out
//! of the export (`extract/emit/item.ts`).

use crate::observation::Observation;
use crate::observers::item::npc_drops::NpcDrop;
use crate::observers::item::object_drops::ObjectDrop;
use indexmap::IndexMap;
use std::collections::BTreeSet;

/// Items that can drop from any NPC or object: their drop sources are never exported. Their lists
/// kept growing and caused problems in Questie's DB source mode.
#[rustfmt::skip]
const WORLD_DROP_ITEM_IDS: [i64; 53] = [
    // Waylaid Crates
    248682, 248683, 248684, 248685, 248686, 248687, 248688, 248689, 248690, 248691, 248692, 248693,
    248694, 248695, 248696, 248697, 248698, 248699, 248702, 248703, 248704, 248705, 248706, 248707,
    248708, 248709, 248710, 248711, 248712, 248713,
    // Scrolls with scrambled names
    211780, 211784, 211785, 211786, 211787, 211853, 211854, 211855, 213543, 213544, 213545, 213546,
    213547, 281015, 281016, 281017, 281018,
    // Sealed Writs
    262888, 262889, 262890,
    // Pristine Leather / Hide
    249427, 249428,
    // Mote of Unstable Magic
    274964,
];

/// The merged drops of every item but the world-drop ones.
pub fn without_world_drops(drops: IndexMap<i64, Vec<i64>>) -> IndexMap<i64, Vec<i64>> {
    drops.into_iter().filter(|(item_id, _)| !WORLD_DROP_ITEM_IDS.contains(item_id)).collect()
}

pub fn merge_npc_drops(observations: &[Observation<NpcDrop>]) -> Vec<i64> {
    let ids: BTreeSet<i64> = observations.iter().map(|obs| obs.value.npc_id).collect();
    ids.into_iter().collect()
}

pub fn merge_object_drops(observations: &[Observation<ObjectDrop>]) -> Vec<i64> {
    let ids: BTreeSet<i64> = observations.iter().map(|obs| obs.value.object_id).collect();
    ids.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::Confidence;

    #[test]
    fn leaves_out_the_drops_of_world_drop_items() {
        let world_drops = [248682, 248713, 211780, 211786, 281018, 262889, 249427, 249428, 274964];
        let drops: IndexMap<i64, Vec<i64>> = world_drops.into_iter().chain([12345]).map(|id| (id, vec![2])).collect();
        assert_eq!(without_world_drops(drops).into_iter().collect::<Vec<_>>(), [(12345, vec![2])]);
    }

    fn npc(npc_id: i64, t: f64) -> Observation<NpcDrop> {
        Observation::new(12345, NpcDrop { npc_id }, Confidence::High, t)
    }

    fn object(object_id: i64, t: f64) -> Observation<ObjectDrop> {
        Observation::new(12345, ObjectDrop { object_id }, Confidence::High, t)
    }

    #[test]
    fn npc_drops_union_npc_ids_across_observations() {
        assert_eq!(merge_npc_drops(&[npc(823, 5.0), npc(824, 5.0), npc(823, 10.0)]), [823, 824]);
    }

    #[test]
    fn npc_drops_of_no_observations_are_empty() {
        assert_eq!(merge_npc_drops(&[]), Vec::<i64>::new());
    }

    #[test]
    fn npc_drops_are_sorted_ascending() {
        assert_eq!(merge_npc_drops(&[npc(900, 5.0), npc(100, 5.0), npc(500, 5.0)]), [100, 500, 900]);
    }

    #[test]
    fn object_drops_union_object_ids_across_observations() {
        assert_eq!(merge_object_drops(&[object(2843, 5.0), object(2844, 5.0), object(2843, 10.0)]), [2843, 2844]);
    }

    #[test]
    fn object_drops_of_no_observations_are_empty() {
        assert_eq!(merge_object_drops(&[]), Vec::<i64>::new());
    }

    #[test]
    fn object_drops_are_sorted_ascending() {
        assert_eq!(merge_object_drops(&[object(900, 5.0), object(100, 5.0), object(500, 5.0)]), [100, 500, 900]);
    }
}
