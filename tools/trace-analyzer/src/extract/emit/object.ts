// Fact -> named record for the object entity kind. See emit/npc.ts for the general shape.

import type { Fact } from "../aggregate";
import { emitRecords } from "./records";

export interface ObjectFacts {
  [field: string]: Map<number, Fact<unknown>>;
  name: Map<number, Fact<string>>;
  spawns: Map<number, Fact<Record<number, Array<[number, number]>>>>;
  zoneID: Map<number, Fact<number>>;
  questStarts: Map<number, Fact<number[]>>;
  questEnds: Map<number, Fact<number[]>>;
}

export function emitObjectRecords(facts: ObjectFacts): Map<number, Record<string, unknown>> {
  return emitRecords(facts);
}
