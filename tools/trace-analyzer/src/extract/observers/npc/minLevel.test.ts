import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeMinLevel } from "./minLevel";

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

describe("observeMinLevel (npc)", () => {
  it("should always return no observations (no addon-side NPC level signal exists yet)", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitLevel: { player: [{ t: 0, tp: 0, v: 5 }] },
    });

    expect(observeMinLevel(session)).toEqual([]);
  });
});
