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
  it("should derive requiredLevel as the lowest questLevel for a quest", () => {
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
        provenance: { session: "test-session", t: 0 },
      },
    ]);
  });

  it("should return no observations when no questLevel data exists", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
    });

    expect(observeRequiredLevel(session)).toEqual([]);
  });

  it("should compute the minimum across multiple quests in the same session", () => {
    const session = makeSession({
      GetQuestID: [
        { t: 3, tp: 3, v: 96659 },
        { t: 7, tp: 7, v: 12345 },
      ],
      GetQuestLogTitle: {
        "96659": [
          { t: 3, tp: 3, v: { 1: "A Threat Within", 2: 60, 8: 96659, n: 8 } },
        ],
        "12345": [
          { t: 7, tp: 7, v: { 1: "Arya's Request", 2: 45, 8: 12345, n: 8 } },
        ],
      },
    });

    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 60,
        confidence: "medium",
        provenance: { session: "test-session", t: 0 },
      },
      {
        entityId: 12345,
        value: 45,
        confidence: "medium",
        provenance: { session: "test-session", t: 0 },
      },
    ]);
  });

  it("should derive requiredLevel from C_QuestLog.GetInfo when available", () => {
    const session = makeSession({
      GetQuestID: [
        { t: 3, tp: 3, v: 96659 },
        { t: 7, tp: 7, v: 96659 },
      ],
      "C_QuestLog.GetInfo": {
        "96659": [
          { t: 3, tp: 3, v: { questID: 96659, level: 60, title: "A Threat Within" } },
          { t: 7, tp: 7, v: { questID: 96659, level: 50, title: "A Threat Within" } },
        ],
      },
    });

    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 50,
        confidence: "medium",
        provenance: { session: "test-session", t: 0 },
      },
    ]);
  });
});
