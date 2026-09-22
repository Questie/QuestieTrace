import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { objectEncounters } from "./_encounters";

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

describe("objectEncounters", () => {
  it("should emit one encounter per GameObject-kind GUID observation across all tracked tokens", () => {
    const session = makeSession({
      UnitGUID: {
        target: [{ t: 0, tp: 0, v: "GameObject-0-5208-0-7-2843-0000399" }],
      },
    });

    expect(objectEncounters(session)).toEqual([{ objectID: 2843, token: "target", t: 0 }]);
  });

  it("should ignore Creature/Player GUIDs seen on the same tokens", () => {
    const session = makeSession({
      UnitGUID: {
        target: [
          { t: 0, tp: 0, v: "Creature-0-5208-0-7-823-000031" },
          { t: 1, tp: 1, v: "Player-5284-0362" },
        ],
      },
    });

    expect(objectEncounters(session)).toEqual([]);
  });

  it("should return an empty array when no UnitGUID stream exists", () => {
    expect(objectEncounters(makeSession({}))).toEqual([]);
  });
});
