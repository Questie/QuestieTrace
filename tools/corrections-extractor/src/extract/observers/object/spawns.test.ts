import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeSpawns } from "./spawns";

function makeSession(functions: SessionRecord["functions"], events: SessionRecord["events"] = []): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events,
    functions,
    functionsDelta: {},
  };
}

describe("observeSpawns (object)", () => {
  it("should convert normalized map coordinates to Questie percentages", () => {
    const session = makeSession({
      UnitGUID: {
        target: [
          { t: 10, tp: 10, v: "GameObject-0-5208-0-7-2843-0000399" },
        ],
      },
      "C_Map.GetBestMapForUnit": {
        player: [{ t: 10, tp: 10, v: 1436 }], // uiMapID 1436 = Westfall (area 40)
      },
      "C_Map.GetPlayerMapPosition": {
        player: [{ t: 10, tp: 10, v: { x: 0.5611, y: 0.6164 } }],
      },
    });

    expect(observeSpawns(session)).toEqual([
      {
        entityId: 2843,
        value: { zoneID: 40, x: 56.11, y: 61.64 },
        confidence: "high",
        provenance: { session: "test-session", t: 10 },
        token: "target",
      },
    ]);
  });

  it("should skip encounters where player zone is unknown", () => {
    const session = makeSession({
      UnitGUID: {
        target: [
          { t: 10, tp: 10, v: "GameObject-0-5208-0-7-2843-0000399" },
        ],
      },
      "C_Map.GetBestMapForUnit": {
        player: [],
      },
      "C_Map.GetPlayerMapPosition": {
        player: [{ t: 10, tp: 10, v: { x: 0.3001, y: 0.8602 } }],
      },
    });

    expect(observeSpawns(session)).toEqual([]);
  });

  it("should skip encounters where player position is unknown", () => {
    const session = makeSession({
      UnitGUID: {
        target: [
          { t: 10, tp: 10, v: "GameObject-0-5208-0-7-2843-0000399" },
        ],
      },
      "C_Map.GetBestMapForUnit": {
        player: [{ t: 10, tp: 10, v: 1436 }],
      },
      "C_Map.GetPlayerMapPosition": {
        player: [],
      },
    });

    expect(observeSpawns(session)).toEqual([]);
  });

  it("should return empty array when no object encounters exist", () => {
    const session = makeSession({
      UnitGUID: {},
      "C_Map.GetBestMapForUnit": {},
      "C_Map.GetPlayerMapPosition": {},
    });

    expect(observeSpawns(session)).toEqual([]);
  });

  it("should observe a looted object's spawn at the player's position with the loot token", () => {
    const session = makeSession({
      GetLootSourceInfo: { 1: [{ t: 5, tp: 5, v: { 1: "GameObject-0-5208-0-7-673206-0000399", 2: 1, n: 2 } }] },
      "C_Map.GetBestMapForUnit": { player: [{ t: 0, tp: 0, v: 1436 }] }, // Westfall -> area 40
      "C_Map.GetPlayerMapPosition": { player: [{ t: 0, tp: 0, v: { x: 0.5611, y: 0.6164 } }] },
    }, [{ t: 5, tp: 5, e: "LOOT_READY", a: { n: 0 } }]);

    expect(observeSpawns(session)).toEqual([
      {
        entityId: 673206,
        value: { zoneID: 40, x: 56.11, y: 61.64 },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
        token: "loot",
      },
    ]);
  });
});
