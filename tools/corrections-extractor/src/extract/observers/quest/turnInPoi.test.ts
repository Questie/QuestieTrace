import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeTurnInPoi, turnInPoiSpawns } from "./turnInPoi";

const BARRENS_UI_MAP = 1413;
const BARRENS_AREA = 17;

function makeSession(functions: SessionRecord["functions"]): SessionRecord {
  return {
    schemaVersion: 14,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    events: [],
    functions,
    functionsDelta: {},
  };
}

function poi(questID: number, mapID: number, x: number, y: number, extra: Record<string, unknown> = {}) {
  return { questID, mapID, x, y, isQuestStart: false, inProgress: true, ...extra };
}

describe("observeTurnInPoi", () => {
  it("should observe the POI position of a complete quest", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestsOnMap": {
        [BARRENS_UI_MAP]: [{ t: 5, tp: 5, v: [poi(891, BARRENS_UI_MAP, 0.6228, 0.3903)] }],
      },
      "C_QuestLog.IsComplete": { 891: [{ t: 0, tp: 0, v: true }] },
    });

    expect(observeTurnInPoi(session)).toEqual([
      {
        entityId: 891,
        value: { zoneID: BARRENS_AREA, x: 62.28, y: 39.03 },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
    ]);
  });

  it("should ignore POIs of incomplete quests", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestsOnMap": {
        [BARRENS_UI_MAP]: [{ t: 5, tp: 5, v: [poi(891, BARRENS_UI_MAP, 0.62, 0.39)] }],
      },
      "C_QuestLog.IsComplete": { 891: [{ t: 0, tp: 0, v: false }, { t: 10, tp: 10, v: true }] },
    });

    expect(observeTurnInPoi(session)).toEqual([]);
  });

  it("should ignore POIs reported on a different map than queried", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestsOnMap": {
        [BARRENS_UI_MAP]: [{ t: 5, tp: 5, v: [poi(891, 1414, 0.62, 0.39)] }],
      },
      "C_QuestLog.IsComplete": { 891: [{ t: 0, tp: 0, v: true }] },
    });

    expect(observeTurnInPoi(session)).toEqual([]);
  });

  it("should ignore a lagging POI still showing the objective location after completion", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestsOnMap": {
        [BARRENS_UI_MAP]: [
          { t: 5, tp: 5, v: [poi(891, BARRENS_UI_MAP, 0.3, 0.3)] },
          { t: 10, tp: 10, v: [poi(891, BARRENS_UI_MAP, 0.3, 0.3), poi(1, BARRENS_UI_MAP, 0.1, 0.1)] },
          { t: 10.35, tp: 10.35, v: [poi(891, BARRENS_UI_MAP, 0.62, 0.39)] },
        ],
      },
      "C_QuestLog.IsComplete": { 891: [{ t: 0, tp: 0, v: false }, { t: 10, tp: 10, v: true }] },
    });

    expect(observeTurnInPoi(session).map((o) => o.value)).toEqual([{ zoneID: BARRENS_AREA, x: 62, y: 39 }]);
  });

  it("should ignore POIs on maps without a Questie zone", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestsOnMap": {
        1414: [{ t: 5, tp: 5, v: [poi(891, 1414, 0.62, 0.39)] }],
      },
      "C_QuestLog.IsComplete": { 891: [{ t: 0, tp: 0, v: true }] },
    });

    expect(observeTurnInPoi(session)).toEqual([]);
  });

  it("should ignore quest start POIs", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestsOnMap": {
        [BARRENS_UI_MAP]: [{ t: 5, tp: 5, v: [poi(891, BARRENS_UI_MAP, 0.62, 0.39, { isQuestStart: true })] }],
      },
      "C_QuestLog.IsComplete": { 891: [{ t: 0, tp: 0, v: true }] },
    });

    expect(observeTurnInPoi(session)).toEqual([]);
  });

  it("should deduplicate repeated samples of the same POI", () => {
    const sample = [poi(891, BARRENS_UI_MAP, 0.62, 0.39)];
    const session = makeSession({
      "C_QuestLog.GetQuestsOnMap": {
        [BARRENS_UI_MAP]: [{ t: 5, tp: 5, v: sample }, { t: 6, tp: 6, v: [...sample, poi(1, BARRENS_UI_MAP, 0.1, 0.1)] }],
      },
      "C_QuestLog.IsComplete": { 891: [{ t: 0, tp: 0, v: true }] },
    });

    expect(observeTurnInPoi(session)).toHaveLength(1);
  });
});

describe("turnInPoiSpawns", () => {
  const poiObs = {
    entityId: 891,
    value: { zoneID: BARRENS_AREA, x: 62.28, y: 39.03 },
    confidence: "high" as const,
    provenance: { session: "s", t: 5 },
  };

  it("should attribute the POI to a single creature finisher", () => {
    const finishedBy = new Map([[891, { value: { creatures: [3453], objects: [] } }]]);

    expect(turnInPoiSpawns([poiObs], finishedBy)).toEqual({
      npcs: [{ ...poiObs, entityId: 3453, token: "questpoi" }],
      objects: [],
    });
  });

  it("should attribute the POI to a single object finisher", () => {
    const finishedBy = new Map([[891, { value: { creatures: [], objects: [9000] } }]]);

    expect(turnInPoiSpawns([poiObs], finishedBy)).toEqual({
      npcs: [],
      objects: [{ ...poiObs, entityId: 9000, token: "questpoi" }],
    });
  });

  it("should skip quests with ambiguous or unknown finishers", () => {
    const finishedBy = new Map([[891, { value: { creatures: [1, 2], objects: [] } }]]);

    expect(turnInPoiSpawns([poiObs], finishedBy)).toEqual({ npcs: [], objects: [] });
    expect(turnInPoiSpawns([poiObs], new Map())).toEqual({ npcs: [], objects: [] });
  });
});
