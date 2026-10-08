// Shared helper for quest field observers: not a Questie DB field itself, just
// the common "when did we see this questID" anchor list that individual field
// observers probe other streams at (see Modules/Trackers/QuestDialog.lua -
// GetQuestID() is sampled alongside GetTitleText()/GetQuestText()/etc. every
// time a quest-related frame opens/updates/closes).
//
// Unlike npc/object, the entity id comes directly from the stream's own value
// - no GUID parsing needed.

import { getStream } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";

export interface QuestEncounter {
  questID: number;
  t: number;
}

/**
 * Every point in time GetQuestID() returned a nonzero (active quest frame)
 * value. The same questID can appear multiple times - `aggregate.ts` is
 * responsible for merging duplicates.
 */
export function questEncounters(session: SessionRecord): QuestEncounter[] {
  const stream = getStream(session, "GetQuestID");
  if (!stream) return [];

  const out: QuestEncounter[] = [];
  for (const entry of stream) {
    if (typeof entry.v === "number" && entry.v !== 0) {
      out.push({ questID: entry.v, t: entry.t });
    }
  }
  return out;
}
