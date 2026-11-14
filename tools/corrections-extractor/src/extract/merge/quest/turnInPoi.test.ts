import { describe, expect, it } from "vitest";
import { turnInPoiSpawns } from "./turnInPoi";

const BARRENS_AREA = 17;

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
