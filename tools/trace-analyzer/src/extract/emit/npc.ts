// Fact -> named record for the npc entity kind.
//
// Only sets the fields that were actually extracted; `lua-writer.ts` fills the
// rest from the npcKeys schema defaults. As more npc field observers are added
// (see observers/npc/), this function grows to read their aggregated Facts too -
// no other code needs to change.

import type { Fact } from "../aggregate";

export interface NpcFacts {
  name: Map<number, Fact<string>>;
  minLevel: Map<number, Fact<number>>;
  maxLevel: Map<number, Fact<number>>;
}

export function emitNpcRecords(facts: NpcFacts): Map<number, Record<string, unknown>> {
  const ids = new Set<number>([...facts.name.keys(), ...facts.minLevel.keys(), ...facts.maxLevel.keys()]);

  const records = new Map<number, Record<string, unknown>>();
  for (const id of ids) {
    const record: Record<string, unknown> = {};
    const name = facts.name.get(id);
    if (name) record.name = name.value;
    const minLevel = facts.minLevel.get(id);
    if (minLevel) record.minLevel = minLevel.value;
    const maxLevel = facts.maxLevel.get(id);
    if (maxLevel) record.maxLevel = maxLevel.value;
    records.set(id, record);
  }
  return records;
}
