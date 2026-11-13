// Shared by the startedBy/finishedBy merges. A giver seen in few sessions is more
// likely a misattributed one (e.g. a stale questnpc) than a real one, and a wrong
// giver does more harm than a missing one. The same holds for a giver seen in a tiny
// share of the sessions of the quest's strongest giver, e.g. an object that only
// replaces the real starter after the previous quest was turned in.

import type { Observation } from "../../observation";

export const MIN_GIVER_SESSIONS = 5;
export const MIN_GIVER_SHARE = 0.005;

/**
 * Per `idOf` (one per giver type), the ids it returns, ascending, that were seen in at least
 * MIN_GIVER_SESSIONS sessions and in at least MIN_GIVER_SHARE of the strongest giver's sessions.
 */
export function supportedIds<T>(observations: Observation<T>[], ...idOfs: Array<(value: T) => number | undefined>): number[][] {
  const sessionsByType = idOfs.map((idOf) => {
    const sessionsById = new Map<number, Set<string>>();
    for (const obs of observations) {
      const id = idOf(obs.value);
      if (!id) continue;
      const sessions = sessionsById.get(id) ?? new Set<string>();
      sessions.add(obs.provenance.sessionKey ?? obs.provenance.session);
      sessionsById.set(id, sessions);
    }
    return sessionsById;
  });
  const strongest = Math.max(0, ...sessionsByType.flatMap((sessionsById) => [...sessionsById.values()].map((s) => s.size)));
  return sessionsByType.map((sessionsById) =>
    [...sessionsById]
      .filter(([, sessions]) => sessions.size >= MIN_GIVER_SESSIONS && sessions.size >= strongest * MIN_GIVER_SHARE)
      .map(([id]) => id)
      .sort((a, b) => a - b),
  );
}
