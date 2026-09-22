// Thin object-specific wrapper over the shared observers/_guidEncounters.ts helper.
// Not a Questie DB field itself, just the common "when/where did we see this
// objectID" anchor list that individual object field observers probe other
// streams at (e.g. WoW lets "target" resolve to a GameObject when interacting
// with a lootable chest, so the same UnitGUID/UnitName streams npc uses apply here).

import type { SessionRecord } from "../../../core/types";
import { guidEncounters } from "../_guidEncounters";

export interface ObjectEncounter {
  objectID: number;
  /** The unit token this encounter came from, so field observers can read UnitName(token) etc. at the same t. */
  token: string;
  t: number;
}

export function objectEncounters(session: SessionRecord): ObjectEncounter[] {
  return guidEncounters(session, "object").map((e) => ({ objectID: e.entityId, token: e.token, t: e.t }));
}
