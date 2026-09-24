import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeZoneOrSort } from "./zoneOrSort";

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

describe("observeZoneOrSort", () => {
  it("should map a quest-log zone header to its AreaTable ID", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      QuestLogZone: {
        "96659": [{ t: 2, tp: 2, v: "Westfall" }],
      },
    });

    expect(observeZoneOrSort(session)).toEqual([
      {
        entityId: 96659,
        value: 40,
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should map a quest-log special header to a negative QuestSort ID", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      QuestLogZone: {
        "96659": [{ t: 2, tp: 2, v: "Epic" }],
      },
    });

    expect(observeZoneOrSort(session)[0]?.value).toBe(-1);
  });

  it("should skip unknown or missing header titles", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      QuestLogZone: {
        "96659": [{ t: 2, tp: 2, v: "Not a DB2 header" }],
      },
    });

    expect(observeZoneOrSort(session)).toEqual([]);
  });
});
