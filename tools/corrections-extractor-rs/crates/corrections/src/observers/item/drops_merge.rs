//! The merges of item.npcDrops and item.objectDrops. Port of
//! `extract/observers/item/dropsMerge.ts`.
//!
//! Each observation carries one source id; the merge unions them per item into ids
//! ascending without duplicates (written out as a JS `number[]`).

use crate::observation::Observation;
use crate::observers::item::npc_drops::NpcDrop;
use crate::observers::item::object_drops::ObjectDrop;
use std::collections::BTreeSet;

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
