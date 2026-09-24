// questKeys.name - Questie field 1.
// Only names from enUS locale are extracted. Other locales will be handled separately.

import { getLocaleAt, questInfoTitleAt, questTitleAt } from "../../probes";
import { getParamKeys, getStream } from "../../../core/emulator";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { questEncounters } from "./_encounters";

export const observeName: FieldObserver<string> = (session) => {
  const observations: Observation<string>[] = [];
  const questIDsWithDialogTitle = new Set<number>();

  // Quest-dialog titles are the most direct source and retain precedence over
  // the quest-log API when both are available for the same quest.
  for (const encounter of questEncounters(session)) {
    const locale = getLocaleAt(session, encounter.t);
    if (locale !== "enUS") {
      continue; // Skip non-enUS names
    }
    const title = questTitleAt(session, encounter.t);
    if (title) {
      questIDsWithDialogTitle.add(encounter.questID);
      observations.push({
        entityId: encounter.questID,
        value: title,
        confidence: "high",
        provenance: { session: sessionLabel(session), t: encounter.t },
      });
    }
  }

  // C_QuestLog.GetInfo is keyed by questID and is also recorded for quests in
  // the log that never open a quest dialog. It is therefore a necessary source
  // in its own right, not only a fallback for a GetQuestID encounter.
  const infoRoot = session.functions["C_QuestLog.GetInfo"];
  if (!infoRoot || Array.isArray(infoRoot)) return observations;

  for (const questIdKey of getParamKeys(infoRoot)) {
    const questID = Number(questIdKey);
    if (!Number.isFinite(questID) || questIDsWithDialogTitle.has(questID)) continue;

    const stream = getStream(session, "C_QuestLog.GetInfo", questIdKey) ?? [];
    for (const entry of stream) {
      const locale = getLocaleAt(session, entry.t);
      if (locale !== "enUS") continue; // Skip non-enUS names

      const title = questInfoTitleAt(session, questID, entry.t);
      if (title) {
        observations.push({
          entityId: questID,
          value: title,
          confidence: "high",
          provenance: { session: sessionLabel(session), t: entry.t },
        });
      }
    }
  }

  return observations;
};
