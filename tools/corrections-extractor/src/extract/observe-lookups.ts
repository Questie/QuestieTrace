// Which lookup (translation) observers run on a session, and the staged observations they produce.
//
// Kept apart from the merge code for the same reason as observe.ts: the observer version
// (store/observer-version.ts) hashes this file.

import type { SessionRecord } from "../core/types";
import type { Localized, Observation } from "./observation";
import { isForeverSession } from "./observe";
import { observeLocalizedName as observeItemName } from "./observers/item/name";
import { observeLocalizedName as observeNpcName } from "./observers/npc/name";
import { observeLocalizedName as observeObjectName } from "./observers/object/name";
import { observeLocalizedName as observeQuestName, observeLocalizedObjectivesText } from "./observers/quest";
import { sessionServerTime } from "./probes";

export const LOOKUP_OBSERVERS = {
  npcName: observeNpcName,
  questName: observeQuestName,
  objectivesText: observeLocalizedObjectivesText,
  itemName: observeItemName,
  objectName: observeObjectName,
};

export type LookupObserverKey = keyof typeof LOOKUP_OBSERVERS;

/** One session's lookup observations, ready to be added to an accumulator. */
export interface StagedLookupSession {
  /** Stable key of the session (core/trace-data/session-key.ts); a per-run "#<n>" is used when unknown. */
  sessionKey?: string;
  contributorId?: string;
  sessionTime: number | null;
  observations: [LookupObserverKey, Observation<Localized<unknown>>[]][];
}

/** Runs every lookup observer over one session; undefined for sessions that are not observed. */
export function observeLookupSession(session: SessionRecord): StagedLookupSession | undefined {
  if (!isForeverSession(session)) return undefined;
  return {
    sessionTime: sessionServerTime(session),
    observations: (Object.keys(LOOKUP_OBSERVERS) as LookupObserverKey[]).map((key) => [key, LOOKUP_OBSERVERS[key](session) as Observation<Localized<unknown>>[]]),
  };
}
