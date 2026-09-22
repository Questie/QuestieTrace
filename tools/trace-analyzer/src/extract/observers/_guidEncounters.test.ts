import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../core/types";
import { guidEncounters } from "./_guidEncounters";

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

describe("guidEncounters", () => {
  it("should emit one encounter per GUID observation of the requested kind, across all tracked tokens", () => {
    const session = makeSession({
      UnitGUID: {
        target: [{ t: 0, tp: 0, v: "Creature-0-5208-0-7-823-000031" }],
        npc: [{ t: 5, tp: 5, v: "Creature-0-5208-0-7-197-000032" }],
        questnpc: [{ t: 10, tp: 10, v: "Creature-0-5208-0-7-197-000032" }],
      },
    });

    expect(guidEncounters(session, "npc")).toEqual([
      { entityId: 823, token: "target", t: 0 },
      { entityId: 197, token: "npc", t: 5 },
      { entityId: 197, token: "questnpc", t: 10 },
    ]);
  });

  it("should only match the requested kind, ignoring other GUID kinds on the same tokens", () => {
    const session = makeSession({
      UnitGUID: {
        target: [
          { t: 0, tp: 0, v: "GameObject-0-5208-0-7-2843-0000399" },
          { t: 1, tp: 1, v: "Player-5284-0362" },
          { t: 2, tp: 2, v: "Creature-0-5208-0-7-823-000031" },
        ],
      },
    });

    expect(guidEncounters(session, "object")).toEqual([{ entityId: 2843, token: "target", t: 0 }]);
    expect(guidEncounters(session, "npc")).toEqual([{ entityId: 823, token: "target", t: 2 }]);
  });

  it("should return an empty array when no UnitGUID stream exists", () => {
    expect(guidEncounters(makeSession({}), "npc")).toEqual([]);
  });
});
