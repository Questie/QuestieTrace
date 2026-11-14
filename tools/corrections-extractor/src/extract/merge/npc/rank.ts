// npcKeys.rank - Questie field 6. Observed in observers/npc/rank.ts.
//
// The classification is static, so sessions can only disagree through a wrong
// pairing. The rank seen in the most sessions wins; a tie or a rank seen in only
// one session gives no value. Rank 0 is not exported, since it adds nothing over nil.

import type { Observation } from "../../observation";

const MIN_SESSIONS = 2;

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
