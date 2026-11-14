// Quest turn-in locations from C_QuestLog.GetQuestsOnMap (QuestPOI tracker).
//
// Once a quest in the log is complete, its POI x/y points at the turn-in
// location. These are not a Questie quest field themselves; at finalize time
// they are attached to the quest's finisher (see finishedBy) as spawn
// observations with the `questpoi` token, which overrides all other spawn
// sources for that npc/object.
//
// Only POIs reported on their own map (poi.mapID === queried uiMapID) are used,
// so the coordinates are relative to the zone they get keyed by.

import { getStream, valueAt } from "../../../core/emulator";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { toQuestieMapPosition } from "../../probes";
import { areaIdForUiMapId } from "../../resources/ui-map-to-area";

export interface TurnInPoiValue {
  zoneID: number;
  x: number;
  y: number;
}

interface QuestPoi {
  questID?: unknown;
  mapID?: unknown;
  x?: unknown;
  y?: unknown;
  isQuestStart?: unknown;
}

export const observeTurnInPoi: FieldObserver<TurnInPoiValue> = (session) => {
  const observations: Observation<TurnInPoiValue>[] = [];
  const byMap = session.functions["C_QuestLog.GetQuestsOnMap"];
  if (!byMap || Array.isArray(byMap)) return observations;

  const seen = new Set<string>();
  for (const mapKey of Object.keys(byMap)) {
    const uiMapID = Number(mapKey);
    const zoneID = areaIdForUiMapId(uiMapID);
    if (zoneID === null) continue;

    // POI updates can lag behind IsComplete, so a POI equal to the last one seen
    // while the quest was incomplete is still the objective location.
    const lastIncompletePos = new Map<number, string>();

    for (const entry of getStream(session, "C_QuestLog.GetQuestsOnMap", mapKey) ?? []) {
      if (!Array.isArray(entry.v)) continue;

      for (const poi of entry.v as QuestPoi[]) {
        const { questID, mapID, x, y } = poi;
        if (typeof questID !== "number" || typeof x !== "number" || typeof y !== "number") continue;
        if (mapID !== uiMapID || poi.isQuestStart === true) continue;
        if (x <= 0 || y <= 0) continue;

        const rawKey = `${x},${y}`;
        const isComplete = valueAt(getStream(session, "C_QuestLog.IsComplete", questID) ?? [], entry.t);
        if (isComplete !== true) {
          lastIncompletePos.set(questID, rawKey);
          continue;
        }
        if (lastIncompletePos.get(questID) === rawKey) continue;

        const pos = toQuestieMapPosition({ x, y });
        const key = `${questID}:${zoneID}:${pos.x},${pos.y}`;
        if (seen.has(key)) continue;
        seen.add(key);

        observations.push({
          entityId: questID,
          value: { zoneID, x: pos.x, y: pos.y },
          confidence: "high",
          provenance: { session: sessionLabel(session), t: entry.t },
        });
      }
    }
  }

  return observations;
};
