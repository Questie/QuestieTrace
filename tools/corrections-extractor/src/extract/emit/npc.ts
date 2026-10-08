// Fact -> named record for the npc entity kind.
//
// Only sets the fields that were actually extracted - this is a sparse
// corrections record (see schema/corrections-writer.ts), not a full DB row;
// fields with no Fact are simply absent, not defaulted. As more npc field
// observers are added (see observers/<entity>/), this function grows to read
// their aggregated Facts too - no other code needs to change.

import type { Fact } from "../aggregate";
import { emitRecords } from "./records";

export interface NpcFacts {
  [field: string]: Map<number, Fact<unknown>>;
  name: Map<number, Fact<string>>;
  minLevel: Map<number, Fact<number>>;
  maxLevel: Map<number, Fact<number>>;
  spawns: Map<number, Fact<Record<number, Array<[number, number]>>>>;
  zoneID: Map<number, Fact<number>>;
  questStarts: Map<number, Fact<number[]>>;
  questEnds: Map<number, Fact<number[]>>;
}

export function emitNpcRecords(facts: NpcFacts): Map<number, Record<string, unknown>> {
  return emitRecords(facts);
}
