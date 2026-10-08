// questKeys.objectivesText - Questie field 8.
// type: table {string, ...}
//
// Questie renders this as the quest's objective text in the quest tooltip
// (`QO.Description = QO.objectivesText` in QuestieDB.GetQuest), so the field is
// the quest's *objectives summary*, not the quest's flavour/description text.
// The matching client source is the second return of GetQuestLogQuestText:
//
//   GetQuestLogQuestText(questLogIndex) -> questDescription, questObjectives
//                                      ^ ignored       ^ this field
//
// Both returns live in one packed tuple stored under the questID, so the
// first element is deliberately dropped. GetObjectiveText (the quest progress
// dialog stream) carries the same text but is a flat, un-keyed stream whose
// quest attribution is only as good as the GetQuestID() sample taken at the
// same instant, so it is not used here.
//
// Questie stores the text pre-split into lines, with the blank lines between
// paragraphs preserved (a two-paragraph summary is `{line1, "", line2}`), so
// the raw string is split on newlines and interior blanks are kept to match
// the real DB rows.

import { emulate, getParamKeys, getStream } from "../../../core/emulator";
import { getLocaleAt } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

/** Questie's objectivesText value: the summary split into lines. */
export type QuestObjectivesTextValue = string[];

/** Return position of `questObjectives` inside the packed GetQuestLogQuestText tuple. */
const OBJECTIVES_TUPLE_INDEX = 1;

/**
 * Split a quest objectives summary into Questie's line list.
 *
 * The client separates paragraphs with a blank line, and Questie's DB rows keep
 * that empty string in place, so interior blanks survive. Only leading/trailing
 * blanks are dropped - a summary that is empty after trimming produces no
 * lines at all (auto-complete quests have no objectivesText).
 */
function splitLines(value: string): string[] {
  const lines = value.split(/\r?\n/).map((line) => line.trim());
  while (lines.length > 0 && lines[0].length === 0) lines.shift();
  while (lines.length > 0 && lines[lines.length - 1].length === 0) lines.pop();
  return lines;
}

function readObjectivesText(value: unknown): string | null {
  const tuple = emulate(value);
  if (!Array.isArray(tuple)) return null;
  const text = tuple[OBJECTIVES_TUPLE_INDEX];
  return typeof text === "string" ? text : null;
}

export const observeObjectivesText: FieldObserver<QuestObjectivesTextValue> = (session) => {
  const observations: Observation<QuestObjectivesTextValue>[] = [];

  const root = session.functions["GetQuestLogQuestText"];
  if (!root || Array.isArray(root)) return observations;

  for (const questIdKey of getParamKeys(root)) {
    const questID = Number(questIdKey);
    if (!Number.isFinite(questID)) continue;

    const stream = getStream(session, "GetQuestLogQuestText", questIdKey) ?? [];
    for (const entry of stream) {
      const locale = getLocaleAt(session, entry.t);
      // Older traces may not contain GetLocale. Explicit non-enUS sessions are
      // still rejected; absent locale is treated as the legacy enUS default.
      if (locale !== null && locale !== "enUS") continue;

      const text = readObjectivesText(entry.v);
      if (text === null) continue;

      const lines = splitLines(text);
      if (lines.length === 0) continue;

      observations.push({
        entityId: questID,
        value: lines,
        // The objectives text is read straight out of the quest log API.
        confidence: "high",
        provenance: { session: sessionLabel(session), t: entry.t },
      });
    }
  }

  return observations;
};
