import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeName } from "./name";

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

describe("observeName (npc)", () => {
  it("should observe the npc's name at the same time/token as the GUID encounter", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Kobold Vermin", n: 2 } }] },
    });

    expect(observeName(session)).toEqual([
      {
        entityId: 823,
        value: "Kobold Vermin",
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should skip encounters where no name was observed", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
    });

    expect(observeName(session)).toEqual([]);
  });
});
