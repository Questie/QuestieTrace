// Fact -> named record for the object entity kind. See emit/npc.ts for the general shape.

import type { Fact } from "../aggregate";
import { emitRecords } from "./records";

export interface ObjectFacts {
  [field: string]: Map<number, Fact<string>>;
  name: Map<number, Fact<string>>;
}

export function emitObjectRecords(facts: ObjectFacts): Map<number, Record<string, unknown>> {
  return emitRecords(facts);
}
