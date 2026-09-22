// Shared helper behind emit/npc.ts, emit/quest.ts, emit/item.ts, emit/object.ts:
// turns a map of { fieldName -> Map<entityId, Fact> } into a map of
// { entityId -> sparse record }, only setting the fields that were actually
// extracted (see schema/corrections-writer.ts - no defaulting happens here or later).

import type { Fact } from "../aggregate";

export function emitRecords<F extends Record<string, Map<number, Fact<any>>>>(
  fieldFacts: F,
): Map<number, Record<string, unknown>> {
  const ids = new Set<number>();
  for (const facts of Object.values(fieldFacts)) {
    for (const id of facts.keys()) ids.add(id);
  }

  const records = new Map<number, Record<string, unknown>>();
  for (const id of ids) {
    const record: Record<string, unknown> = {};
    for (const [fieldName, facts] of Object.entries(fieldFacts)) {
      const fact = facts.get(id);
      if (fact) record[fieldName] = fact.value;
    }
    records.set(id, record);
  }
  return records;
}
