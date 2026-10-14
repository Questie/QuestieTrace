// questKeys.name - Questie field 1.
// observeName keeps enUS names only; observeLocalizedName keeps every locale (for the lookup export).

import { getLocaleAt, questInfoTitleAt, questTitleAt } from "../../probes";
import { getParamKeys, getStream } from "../../../core/emulator";
import { onlyLocale, sessionLabel, type FieldObserver, type Localized, type Observation } from "../../observation";
import { questEncounters } from "./_encounters";

export const observeLocalizedName: FieldObserver<Localized<string>> = (session) => {
  const observations: Observation<Localized<string>>[] = [];
  const questIDsWithDialogTitle = new Set<number>();

  // Quest-dialog titles are the most direct source and retain precedence over
  // the quest-log API when both are available for the same quest.
  for (const encounter of questEncounters(session)) {
    const locale = getLocaleAt(session, encounter.t);
    if (locale === null) continue;
    const title = questTitleAt(session, encounter.t);
    if (title) {
      questIDsWithDialogTitle.add(encounter.questID);
      observations.push({
        entityId: encounter.questID,
        value: { locale, value: title },
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
      if (locale === null) continue;

      const title = questInfoTitleAt(session, questID, entry.t);
      if (title) {
        observations.push({
          entityId: questID,
          value: { locale, value: title },
          confidence: "high",
          provenance: { session: sessionLabel(session), t: entry.t },
        });
      }
    }
  }

  return observations;
};

export const observeName: FieldObserver<string> = (session) => onlyLocale(observeLocalizedName(session), "enUS");
