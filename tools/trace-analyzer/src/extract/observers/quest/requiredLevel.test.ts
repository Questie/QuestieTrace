import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeRequiredLevel } from "./requiredLevel";

function makeSession(functions: SessionRecord["functions"] = {}): SessionRecord {
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

describe("observeRequiredLevel", () => {
  it("should derive requiredLevel as the lowest acceptance level for a quest", () => {
    const session = makeSession({
      QuestAcceptLevel: {
        "96659": [
          { t: 3, tp: 3, v: 60 },
          { t: 7, tp: 7, v: 55 },
        ],
      },
    });

    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 55,
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should return no observations when no QuestAcceptLevel data exists", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
    });

    expect(observeRequiredLevel(session)).toEqual([]);
  });

  it("should compute the minimum across multiple quests in the same session", () => {
    const session = makeSession({
      QuestAcceptLevel: {
        "96659": [{ t: 3, tp: 3, v: 60 }],
        "12345": [{ t: 7, tp: 7, v: 45 }],
      },
    });

    const result = observeRequiredLevel(session);
    expect(result).toHaveLength(2);
    // Order doesn't matter - check both quests are present
    expect(result.find(r => r.entityId === 96659)).toEqual({
      entityId: 96659,
      value: 60,
      confidence: "high",
      provenance: { session: "test-session", t: 3 },
    });
    expect(result.find(r => r.entityId === 12345)).toEqual({
      entityId: 12345,
      value: 45,
      confidence: "high",
      provenance: { session: "test-session", t: 7 },
    });
  });

  it("should fall back to questLevel when QuestAcceptLevel is not available (legacy traces)", () => {
    const session = makeSession({
      GetQuestID: [
        { t: 3, tp: 3, v: 96659 },
        { t: 7, tp: 7, v: 96659 },
      ],
      GetQuestLogTitle: {
        "96659": [
          { t: 3, tp: 3, v: { 1: "A Threat Within", 2: 60, 8: 96659, n: 8 } },
          { t: 7, tp: 7, v: { 1: "A Threat Within", 2: 55, 8: 96659, n: 8 } },
        ],
      },
    });

    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 55,
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });
});
