import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeSpawns } from "./spawns";

function makeSession(functions: SessionRecord["functions"]): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events: [],
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
        player: [{ t: 10, tp: 10, v: 40 }],
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
        player: [{ t: 10, tp: 10, v: 40 }],
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
});
