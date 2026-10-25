// Pairs an NPC encounter on the "target"/"mouseover" token with a value that
// Modules/Trackers/UnitState.lua sampled for that token (UnitLevel,
// UnitReaction, ...). Both the UnitGUID and the UnitState streams are only
// written on change, so a value is only trusted at a GUID sample when it was
// recorded at the same time, or when PLAYER_TARGET_CHANGED/UPDATE_MOUSEOVER_UNIT
// fired at that time (UnitState samples every token on both, so the last value
// is still current). Other GUID samples (quest/merchant/loot events) are not
// paired. A failed API call at such an event would leave the previous unit's
// value in place; the field merges have to guard against that.

import { getStream, valueAt } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { guidEncounters } from "../_guidEncounters";

const UNIT_STATE_TOKENS = ["target", "mouseover"] as const;
const UNIT_STATE_SAMPLE_EVENTS = new Set(["PLAYER_TARGET_CHANGED", "UPDATE_MOUSEOVER_UNIT"]);

export interface UnitStateEncounter {
  npcID: number;
  token: string;
  t: number;
  value: unknown;
}

/**
 * Every NPC encounter with the paired UnitState value. `streamPath` is the stream
 * name plus any leading arguments before the unit token, e.g. ["UnitLevel"] or
 * ["UnitReaction", "player"].
 */
export function unitStateEncounters(session: SessionRecord, ...streamPath: [string, ...string[]]): UnitStateEncounter[] {
  const out: UnitStateEncounter[] = [];
  const sampleEventTimes = session.events.filter((event) => UNIT_STATE_SAMPLE_EVENTS.has(event.e)).map((event) => event.t);
  for (const token of UNIT_STATE_TOKENS) {
    const stream = getStream(session, ...streamPath, token) ?? [];
    const sampleTimes = new Set([...sampleEventTimes, ...stream.map((entry) => entry.t)]);
    for (const encounter of guidEncounters(session, "npc", [token])) {
      if (!sampleTimes.has(encounter.t)) continue;
      const value = valueAt(stream, encounter.t);
      if (value === undefined) continue;
      out.push({ npcID: encounter.entityId, token, t: encounter.t, value });
    }
  }
  return out;
}
