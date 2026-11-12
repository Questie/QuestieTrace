// Shared by the startedBy/finishedBy merges. A giver seen in few sessions is more
// likely a misattributed one (e.g. a stale questnpc) than a real one, and a wrong
// giver does more harm than a missing one.

import type { Observation } from "../../observation";

export const MIN_GIVER_SESSIONS = 5;

/** Ids returned by `idOf`, ascending, that were seen in at least MIN_GIVER_SESSIONS sessions. */
export function supportedIds<T>(observations: Observation<T>[], idOf: (value: T) => number | undefined): number[] {
  const sessionsById = new Map<number, Set<string>>();
  for (const obs of observations) {
    const id = idOf(obs.value);
    if (!id) continue;
    const sessions = sessionsById.get(id) ?? new Set<string>();
    sessions.add(obs.provenance.sessionKey ?? obs.provenance.session);
    sessionsById.set(id, sessions);
  }
  return [...sessionsById]
    .filter(([, sessions]) => sessions.size >= MIN_GIVER_SESSIONS)
    .map(([id]) => id)
    .sort((a, b) => a - b);
}
