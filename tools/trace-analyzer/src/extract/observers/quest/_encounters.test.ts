import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { questEncounters } from "./_encounters";

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

describe("questEncounters", () => {
  it("should emit one encounter per nonzero GetQuestID observation", () => {
    const session = makeSession({
      GetQuestID: [
        { t: 0, tp: 0, v: 96659 },
        { t: 5, tp: 5, v: 12345 },
      ],
    });

    expect(questEncounters(session)).toEqual([
      { questID: 96659, t: 0 },
      { questID: 12345, t: 5 },
    ]);
  });

  it("should ignore zero (no active quest frame) values", () => {
    const session = makeSession({
      GetQuestID: [
        { t: 0, tp: 0, v: 96659 },
        { t: 5, tp: 5, v: 0 },
      ],
    });

    expect(questEncounters(session)).toEqual([{ questID: 96659, t: 0 }]);
  });

  it("should return an empty array when no GetQuestID stream exists", () => {
    expect(questEncounters(makeSession({}))).toEqual([]);
  });
});
