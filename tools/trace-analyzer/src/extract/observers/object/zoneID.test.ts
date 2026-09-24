import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeZoneID } from "./zoneID";

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

describe("observeZoneID (object)", () => {
  it("should emit spawn observations that can be used to derive zoneID", () => {
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
        player: [{ t: 10, tp: 10, v: { x: 0.3001, y: 0.8602 } }],
      },
    });

    const observations = observeZoneID(session);
    expect(observations).toEqual([
      {
        entityId: 2843,
        value: { zoneID: 40, x: 30.01, y: 86.02 },
        confidence: "high",
        provenance: { session: "test-session", t: 10 },
        token: "target",
      },
    ]);
  });

  it("should return empty array when no data available", () => {
    const session = makeSession({});
    expect(observeZoneID(session)).toEqual([]);
  });
});
