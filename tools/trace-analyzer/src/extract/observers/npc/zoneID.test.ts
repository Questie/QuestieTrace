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

describe("observeZoneID", () => {
  it("should emit spawn observations that can be used to derive zoneID", () => {
    const session = makeSession({
      UnitGUID: {
        npc: [
          { t: 10, tp: 10, v: "Creature-0-5208-0-7-823-000031" },
        ],
      },
      "C_Map.GetBestMapForUnit": {
        player: [{ t: 10, tp: 10, v: 40 }],
      },
      "C_Map.GetPlayerMapPosition": {
        player: [{ t: 10, tp: 10, v: { x: 30.01, y: 86.02 } }],
      },
    });

    const observations = observeZoneID(session);
    expect(observations).toEqual([
      {
        entityId: 823,
        value: { zoneID: 40, x: 30.01, y: 86.02 },
        confidence: "high",
        provenance: { session: "test-session", t: 10 },
        token: "npc",
      },
    ]);
  });

  it("should return empty array when no data available", () => {
    const session = makeSession({});
    expect(observeZoneID(session)).toEqual([]);
  });
});
