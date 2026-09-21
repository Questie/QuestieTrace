// Shared helper for npc field observers: not a Questie DB field itself, just the
// common "when/where did we see this npcID" anchor list that individual field
// observers probe other streams at.

import { getStream } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { parseGuid } from "../../guid";

/** Unit tokens the addon records UnitGUID/UnitName/UnitLevel for (see Modules/Trackers/UnitInteraction.lua). */
const NPC_GUID_TOKENS = ["target", "npc", "questnpc"] as const;

export interface NpcEncounter {
  npcID: number;
  /** The unit token this encounter came from, so field observers can read UnitName(token)/UnitLevel(token) etc. at the same t. */
  token: string;
  t: number;
}

/**
 * Every point in time at which one of the tracked unit tokens resolved to an
 * npc-kind GUID. The same npcID can appear multiple times (different tokens,
 * different times) - `aggregate.ts` is responsible for merging duplicates.
 */
export function npcEncounters(session: SessionRecord): NpcEncounter[] {
  const out: NpcEncounter[] = [];
  for (const token of NPC_GUID_TOKENS) {
    const stream = getStream(session, "UnitGUID", token);
    if (!stream) continue;
    for (const entry of stream) {
      if (typeof entry.v !== "string") continue;
      const parsed = parseGuid(entry.v);
      if (parsed.kind === "npc" && parsed.id !== null) {
        out.push({ npcID: parsed.id, token, t: entry.t });
      }
    }
  }
  return out;
}
