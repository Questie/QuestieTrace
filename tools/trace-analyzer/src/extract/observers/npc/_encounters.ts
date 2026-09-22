// Thin npc-specific wrapper over the shared observers/_guidEncounters.ts helper.
// Not a Questie DB field itself, just the common "when/where did we see this
// npcID" anchor list that individual npc field observers probe other streams at.

import type { SessionRecord } from "../../../core/types";
import { guidEncounters } from "../_guidEncounters";

export interface NpcEncounter {
  npcID: number;
  /** The unit token this encounter came from, so field observers can read UnitName(token)/UnitLevel(token) etc. at the same t. */
  token: string;
  t: number;
}

export function npcEncounters(session: SessionRecord): NpcEncounter[] {
  return guidEncounters(session, "npc").map((e) => ({ npcID: e.entityId, token: e.token, t: e.t }));
}
