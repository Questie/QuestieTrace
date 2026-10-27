// npcKeys.rank - Questie field 6.
// type: int, default: 0
// comment: "see https://github.com/cmangos/issues/wiki/creature_template#rank"
//
// Source: UnitClassification[token] from Modules/Trackers/UnitState.lua, paired
// with the NPC's GUID (see _unitState.ts).
//
// The classification is static, so sessions can only disagree through a wrong
// pairing. The rank seen in the most sessions wins; a tie or a rank seen in only
// one session gives no value. Rank 0 is not exported, since it adds nothing over nil.

import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { unitStateEncounters } from "./_unitState";

const RANK_BY_CLASSIFICATION: Record<string, number> = {
  normal: 0,
  trivial: 0,
  minus: 0,
  elite: 1,
  rareelite: 2,
  worldboss: 3,
  rare: 4,
};

const MIN_SESSIONS = 2;

export const observeRank: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];
  const seen = new Set<string>();
  for (const encounter of unitStateEncounters(session, "UnitClassification")) {
    if (typeof encounter.value !== "string" || !Object.hasOwn(RANK_BY_CLASSIFICATION, encounter.value)) continue;
    const rank = RANK_BY_CLASSIFICATION[encounter.value];
    const key = `${encounter.npcID}:${rank}`;
    if (seen.has(key)) continue;
    seen.add(key);
    observations.push({
      entityId: encounter.npcID,
      value: rank,
      confidence: "high",
      provenance: { session: sessionLabel(session), t: encounter.t },
    });
  }
  return observations;
};

/** Returns undefined when no rank is reliable enough to export or the rank is 0. */
export function mergeRank(observations: Observation<number>[]): number | undefined {
  const sessionsByRank = new Map<number, Set<number | string>>();
  for (const obs of observations) {
    const sessions = sessionsByRank.get(obs.value) ?? new Set<number | string>();
    sessions.add(obs.provenance.sessionKey ?? obs.provenance.session);
    sessionsByRank.set(obs.value, sessions);
  }
  const counts = [...sessionsByRank]
    .map(([rank, sessions]) => ({ rank, count: sessions.size }))
    .sort((a, b) => b.count - a.count);
  const [best, second] = counts;
  if (!best || best.count < MIN_SESSIONS || best.count === second?.count) return undefined;
  return best.rank === 0 ? undefined : best.rank;
}
