// npcKeys.minLevel / npcKeys.maxLevel - Questie fields 4 and 5. Observed in observers/npc/level.ts.
//
// Min and max react to a single wrong level, so a level only counts when at least
// MIN_SESSIONS sessions saw it and it was seen in at least MIN_SHARE of the
// sessions of the NPC's most common level. That drops rare wrong pairings (e.g. 22
// next to 200 sessions of 23-24) but keeps real ranges of spawns with mixed levels.

import type { Observation } from "../../observation";

export interface NpcLevelRange {
  min: number;
  max: number;
}

const MIN_SESSIONS = 2;
const MIN_SHARE = 0.1;

/** Returns undefined when no level was seen often enough. */
export function mergeLevelRange(observations: Observation<number>[]): NpcLevelRange | undefined {
  const sessionsByLevel = new Map<number, Set<number | string>>();
  for (const obs of observations) {
    const sessions = sessionsByLevel.get(obs.value) ?? new Set<number | string>();
    sessions.add(obs.provenance.sessionKey ?? obs.provenance.session);
    sessionsByLevel.set(obs.value, sessions);
  }
  const top = Math.max(...[...sessionsByLevel.values()].map((sessions) => sessions.size));
  const levels = [...sessionsByLevel]
    .filter(([, sessions]) => sessions.size >= MIN_SESSIONS && sessions.size >= top * MIN_SHARE)
    .map(([level]) => level);
  if (levels.length === 0) return undefined;
  return { min: Math.min(...levels), max: Math.max(...levels) };
}
