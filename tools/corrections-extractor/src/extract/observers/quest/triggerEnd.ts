// questKeys.triggerEnd - Questie field 9.
// type: table {text, {[zoneID] = {coordPair,...}}
//
// Event objectives are completed by the player in the world rather than by an
// entity interaction. C_QuestLog.GetQuestObjectives exposes their text and
// finished state; correlate a completed sample with the player's map position
// to obtain the location where the objective was completed. Samples repeat the
// finished state long after the actual completion, so a location is recorded
// only on an objective's incomplete->complete transition - or, when the
// objective is first seen already finished, on that first completed sample.

import { emulate, getParamKeys, getStream } from "../../../core/emulator";
import { getLocaleAt, playerPosAt, playerZoneAt, toQuestieMapPosition } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export type QuestTriggerEndValue = [text: string, locations: Record<number, Array<[number, number]>>];

interface RawObjective {
  type?: unknown;
  text?: unknown;
  finished?: unknown;
}

function readObjectiveList(value: unknown): RawObjective[] {
  const objectives = emulate(value);
  if (!Array.isArray(objectives)) return [];
  return objectives.filter(
    (objective): objective is RawObjective =>
      objective !== null && typeof objective === "object" && !Array.isArray(objective),
  );
}

export const observeTriggerEnd: FieldObserver<QuestTriggerEndValue> = (session) => {
  const observations: Observation<QuestTriggerEndValue>[] = [];
  const objectivesRoot = session.functions["C_QuestLog.GetQuestObjectives"];
  if (!objectivesRoot || Array.isArray(objectivesRoot)) return observations;

  for (const questIdKey of getParamKeys(objectivesRoot)) {
    const questID = Number(questIdKey);
    if (!Number.isFinite(questID)) continue;

    const stream = getStream(session, "C_QuestLog.GetQuestObjectives", questIdKey) ?? [];
    // Objective text -> finished state of the previous positioned sample
    // (undefined = not sampled before). Repeated completed samples are stale
    // re-reads, not completions, so only the transition emits.
    const previousFinished = new Map<string, boolean | undefined>();

    for (const entry of stream) {
      const locale = getLocaleAt(session, entry.t);
      if (locale !== null && locale !== "enUS") continue;

      const zoneID = playerZoneAt(session, entry.t);
      const position = playerPosAt(session, entry.t);
      if (zoneID === null || position === null) continue;

      for (const objective of readObjectiveList(entry.v)) {
        if (objective.type !== "event" || objective.finished === undefined) continue;
        if (typeof objective.text !== "string") continue;
        const text = objective.text.trim();
        if (text.length === 0) continue;

        const finished = objective.finished === true;
        const wasFinished = previousFinished.get(text);
        previousFinished.set(text, finished);

        if (!finished) continue;
        // Emit on the incomplete->complete transition, or on the first
        // completed sample when the objective was never seen incomplete
        // (e.g. the trace starts with the objective already finished).
        if (wasFinished === true) continue;

        const questiePosition = toQuestieMapPosition(position);
        observations.push({
          entityId: questID,
          value: [text, { [zoneID]: [[questiePosition.x, questiePosition.y]] }],
          confidence: "high",
          provenance: { session: sessionLabel(session), t: entry.t },
        });
      }
    }
  }

  return observations;
};
