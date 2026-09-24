// questKeys.triggerEnd - Questie field 9.
// type: table {text, {[zoneID] = {coordPair,...}}
//
// Event objectives are completed by the player in the world rather than by an
// entity interaction. C_QuestLog.GetQuestObjectives exposes their text and
// finished state; correlate a completed sample with the player's map position
// to obtain the location where the objective was completed.

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

function coordinateKey(position: [number, number]): string {
  return `${position[0]},${position[1]}`;
}

/**
 * Merge repeated samples of the same completed event objective into Questie's
 * positional triggerEnd table. Coordinates are de-duplicated per zone because
 * the API is commonly sampled repeatedly around the completion.
 */
export function mergeQuestTriggerEnds(observations: Observation<QuestTriggerEndValue>[]): QuestTriggerEndValue {
  const latestText = [...observations]
    .sort((a, b) => b.provenance.t - a.provenance.t)
    .find((observation) => observation.value[0].trim().length > 0)?.value[0];
  if (latestText === undefined) return ["", {}];

  const locations: Record<number, Array<[number, number]>> = {};
  for (const observation of observations) {
    if (observation.value[0] !== latestText) continue;

    for (const [zoneKey, positions] of Object.entries(observation.value[1])) {
      const zoneID = Number(zoneKey);
      if (!Number.isFinite(zoneID)) continue;
      const zonePositions = locations[zoneID] ?? [];
      const existing = new Set(zonePositions.map(coordinateKey));
      for (const position of positions) {
        const key = coordinateKey(position);
        if (!existing.has(key)) {
          zonePositions.push(position);
          existing.add(key);
        }
      }
      if (zonePositions.length > 0) locations[zoneID] = zonePositions;
    }
  }

  for (const positions of Object.values(locations)) {
    positions.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  }
  return [latestText, locations];
}

export const observeTriggerEnd: FieldObserver<QuestTriggerEndValue> = (session) => {
  const observations: Observation<QuestTriggerEndValue>[] = [];
  const objectivesRoot = session.functions["C_QuestLog.GetQuestObjectives"];
  if (!objectivesRoot || Array.isArray(objectivesRoot)) return observations;

  for (const questIdKey of getParamKeys(objectivesRoot)) {
    const questID = Number(questIdKey);
    if (!Number.isFinite(questID)) continue;

    const stream = getStream(session, "C_QuestLog.GetQuestObjectives", questIdKey) ?? [];
    for (const entry of stream) {
      const locale = getLocaleAt(session, entry.t);
      if (locale !== null && locale !== "enUS") continue;

      const zoneID = playerZoneAt(session, entry.t);
      const position = playerPosAt(session, entry.t);
      if (zoneID === null || position === null) continue;

      for (const objective of readObjectiveList(entry.v)) {
        if (objective.type !== "event" || objective.finished !== true) continue;
        if (typeof objective.text !== "string" || objective.text.trim().length === 0) continue;

        const questiePosition = toQuestieMapPosition(position);
        observations.push({
          entityId: questID,
          value: [objective.text.trim(), { [zoneID]: [[questiePosition.x, questiePosition.y]] }],
          confidence: "high",
          provenance: { session: sessionLabel(session), t: entry.t },
        });
      }
    }
  }

  return observations;
};
