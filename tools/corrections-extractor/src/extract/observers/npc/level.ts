// npcKeys.minLevel / npcKeys.maxLevel - Questie fields 4 and 5.
//
// Source: UnitLevel[token] from Modules/Trackers/UnitState.lua, paired with the
// NPC's GUID (see _unitState.ts). Levels below 1 (-1 is "??", e.g. skull bosses) are skipped.
//
// Min and max react to a single wrong level, so a level only counts when at least
// MIN_SESSIONS sessions saw it and it was seen in at least MIN_SHARE of the
// sessions of the NPC's most common level. That drops rare wrong pairings (e.g. 22
// next to 200 sessions of 23-24) but keeps real ranges of spawns with mixed levels.

import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { unitStateEncounters } from "./_unitState";

export interface NpcLevelRange {
  min: number;
  max: number;
}

const MIN_SESSIONS = 2;
const MIN_SHARE = 0.1;

export const observeLevel: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];
  const seen = new Set<string>();
  for (const encounter of unitStateEncounters(session, "UnitLevel")) {
    const level = encounter.value;
    if (typeof level !== "number" || level < 1) continue;
    const key = `${encounter.npcID}:${level}`;
    if (seen.has(key)) continue;
    seen.add(key);
    observations.push({
      entityId: encounter.npcID,
      value: level,
      confidence: "high",
      provenance: { session: sessionLabel(session), t: encounter.t },
    });
  }
  return observations;
};

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
