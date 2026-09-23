import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeQuestLevel } from "./questLevel";

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

describe("observeQuestLevel", () => {
  it("should read questLevel from GetQuestLogTitle tuple position 2", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      GetQuestLogTitle: {
        "96659": [
          { t: 3, tp: 3, v: { 1: "A Threat Within", 2: 60, 3: 0, 4: false, 5: false, 6: 0, 7: 1, 8: 96659, n: 8 } },
        ],
      },
    });

    expect(observeQuestLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 60,
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should read questLevel from C_QuestLog.GetInfo.level table field", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      "C_QuestLog.GetInfo": {
        "96659": [
          { t: 3, tp: 3, v: { questID: 96659, level: 55, isHeader: false, title: "A Threat Within" } },
        ],
      },
    });

    expect(observeQuestLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 55,
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should prefer C_QuestLog.GetInfo over GetQuestLogTitle when both exist", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      GetQuestLogTitle: {
        "96659": [
          { t: 3, tp: 3, v: { 1: "A Threat Within", 2: 60, 8: 96659, n: 8 } },
        ],
      },
      "C_QuestLog.GetInfo": {
        "96659": [
          { t: 3, tp: 3, v: { questID: 96659, level: 55, isHeader: false, title: "A Threat Within" } },
        ],
      },
    });

    expect(observeQuestLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 55,
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should return no observations when no level stream exists", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
    });

    expect(observeQuestLevel(session)).toEqual([]);
  });

  it("should return no observations when C_QuestLog.GetInfo has no level field", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      "C_QuestLog.GetInfo": {
        "96659": [
          { t: 3, tp: 3, v: { questID: 96659, isHeader: false, title: "A Threat Within" } },
        ],
      },
    });

    expect(observeQuestLevel(session)).toEqual([]);
  });

  it("should skip entries where level is not a finite number", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      GetQuestLogTitle: {
        "96659": [
          { t: 3, tp: 3, v: { 1: "A Threat Within", 2: null, n: 2 } },
        ],
      },
    });

    expect(observeQuestLevel(session)).toEqual([]);
  });

  it("should observe multiple quests in the same session", () => {
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

    expect(observeQuestLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 60,
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
      {
        entityId: 12345,
        value: 45,
        confidence: "high",
        provenance: { session: "test-session", t: 7 },
      },
    ]);
  });
});
