// The objects the player looted: one entry per loot of a game object, read from the
// GetLootSourceInfo(slot) streams (Modules/Trackers/Loot.lua). Looting needs the player
// right next to the object, so the player's position then is the object's position.

import { getParamKeys, getStream } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { parseGuid } from "../../guid";

export interface ObjectLoot {
  objectID: number;
  t: number;
}

export function objectLoots(session: SessionRecord): ObjectLoot[] {
  const sources = session.functions["GetLootSourceInfo"];
  if (!sources) return [];
  // Slots are probed again at LOOT_CLOSED, after the loot: only the samples of an opening are loots.
  const openedAt = new Set(session.events.filter((event) => event.e === "LOOT_READY").map((event) => event.t));

  // A loot fills several slots from the same source: count it once.
  const loots = new Map<string, ObjectLoot>();
  for (const slot of getParamKeys(sources)) {
    for (const entry of getStream(session, "GetLootSourceInfo", slot) ?? []) {
      if (!openedAt.has(entry.t) || !entry.v || typeof entry.v !== "object") continue;
      const source = entry.v as { n?: number; [key: number]: unknown };
      // (guid, quantity) pairs
      for (let i = 1; i <= (source.n ?? 0); i += 2) {
        const guid = source[i];
        if (typeof guid !== "string") continue;
        const parsed = parseGuid(guid);
        if (parsed.kind !== "object" || parsed.id === null) continue;
        loots.set(`${entry.t}:${guid}`, { objectID: parsed.id, t: entry.t });
      }
    }
  }
  return [...loots.values()].sort((a, b) => a.t - b.t || a.objectID - b.objectID);
}
