// Shared helper behind observers/npc/_encounters.ts and observers/object/_encounters.ts:
// walks the tracked UnitGUID unit tokens (see Modules/Trackers/UnitInteraction.lua) and
// emits one "encounter" per point in time a token resolved to a GUID of the requested kind.
//
// npc vs object only differ in which GUID `kind` (see guid.ts) they filter for - both are
// observed on the exact same UnitGUID/UnitName streams (WoW lets "target" etc. resolve to
// a GameObject, e.g. when interacting with a lootable chest).

import { getStream } from "../../core/emulator";
import type { SessionRecord } from "../../core/types";
import { parseGuid, type GuidKind } from "../guid";

/** Unit tokens the addon records UnitGUID/UnitName/UnitLevel for (see Modules/Trackers/UnitInteraction.lua). */
export const TRACKED_UNIT_TOKENS = ["target", "npc", "questnpc"] as const;

export interface GuidEncounter {
  entityId: number;
  /** The unit token this encounter came from, so field observers can read UnitName(token)/UnitLevel(token) etc. at the same t. */
  token: string;
  t: number;
}

/**
 * Every point in time at which one of the tracked unit tokens resolved to a
 * GUID of the requested `kind`. The same entityId can appear multiple times
 * (different tokens, different times) - `aggregate.ts` is responsible for
 * merging duplicates.
 */
export function guidEncounters(
  session: SessionRecord,
  kind: GuidKind,
  tokens: readonly string[] = TRACKED_UNIT_TOKENS,
): GuidEncounter[] {
  const out: GuidEncounter[] = [];
  for (const token of tokens) {
    const stream = getStream(session, "UnitGUID", token);
    if (!stream) continue;
    for (const entry of stream) {
      if (typeof entry.v !== "string") continue;
      const parsed = parseGuid(entry.v);
      if (parsed.kind === kind && parsed.id !== null) {
        out.push({ entityId: parsed.id, token, t: entry.t });
      }
    }
  }
  return out;
}
