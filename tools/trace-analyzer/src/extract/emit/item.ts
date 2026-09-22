// Fact -> named record for the item entity kind. See emit/npc.ts for the general shape.

import type { Fact } from "../aggregate";
import { emitRecords } from "./records";

export interface ItemFacts {
  [field: string]: Map<number, Fact<string>>;
  name: Map<number, Fact<string>>;
}

export function emitItemRecords(facts: ItemFacts): Map<number, Record<string, unknown>> {
  return emitRecords(facts);
}
