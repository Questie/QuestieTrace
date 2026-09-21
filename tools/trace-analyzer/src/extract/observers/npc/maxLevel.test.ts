import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeMaxLevel } from "./maxLevel";

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

describe("observeMaxLevel (npc)", () => {
  it("should observe the npc's level at the same time/token as the GUID encounter", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitLevel: { target: [{ t: 3, tp: 3, v: 6 }] },
    });

    expect(observeMaxLevel(session)).toEqual([
      {
        entityId: 823,
        value: 6,
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should skip encounters where no level was observed", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
    });

    expect(observeMaxLevel(session)).toEqual([]);
  });
});
