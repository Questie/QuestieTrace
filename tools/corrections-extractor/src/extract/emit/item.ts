// Fact -> named record for the item entity kind. See emit/npc.ts for the general shape.

import type { Fact } from "../aggregate";
import { emitRecords } from "./records";

export interface ItemFacts {
  [field: string]: Map<number, Fact<unknown>>;
  name: Map<number, Fact<string>>;
  npcDrops: Map<number, Fact<number[]>>;
  objectDrops: Map<number, Fact<number[]>>;
}

// World-drop items can drop from any NPC/object, so their drop sources are never exported.
const WORLD_DROP_ITEM_IDS: ReadonlySet<number> = new Set([
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
]);

function isWorldDropItem(itemId: number): boolean {
  return WORLD_DROP_ITEM_IDS.has(itemId);
}

function withoutWorldDrops(facts: Map<number, Fact<number[]>>): Map<number, Fact<number[]>> {
  return new Map([...facts].filter(([itemId]) => !isWorldDropItem(itemId)));
}

export function emitItemRecords(facts: ItemFacts): Map<number, Record<string, unknown>> {
  return emitRecords({
    ...facts,
    npcDrops: withoutWorldDrops(facts.npcDrops),
    objectDrops: withoutWorldDrops(facts.objectDrops),
  });
}
