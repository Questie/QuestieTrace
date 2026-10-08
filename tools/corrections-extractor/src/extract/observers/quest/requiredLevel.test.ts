import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeRequiredLevel, mergeRequiredLevel } from "./requiredLevel";

function makeSession(
  functions: SessionRecord["functions"] = {},
  events: SessionRecord["events"] = []
): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events,
    functions,
    functionsDelta: {},
  };
}

function makeQuestAcceptedEvent(t: number, questId: number): SessionRecord["events"][0] {
  return { t, tp: t, e: "QUEST_ACCEPTED", a: { 1: questId, n: 1 } };
}

function makeUnitLevelEntry(t: number, level: number) {
  return { t, tp: t, v: level };
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

  it("should return no observations when no QuestAcceptLevel data exists and no events", () => {
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

  it("should only use QuestAcceptLevel stream, not questLevel", () => {
    const session = makeSession({
      QuestAcceptLevel: {
        "96659": [{ t: 3, tp: 3, v: 55 }],
      },
      GetQuestLogTitle: {
        "96659": [{ t: 3, tp: 3, v: { 1: "A Threat Within", 2: 60, 8: 96659, n: 8 } }],
      },
    });

    // Should use the acceptance level (55), not the quest level (60)
    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 55,
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should fall back to UnitLevel at QUEST_ACCEPTED event time when QuestAcceptLevel is not available", () => {
    const session = makeSession(
      {
        UnitLevel: {
          player: [
            makeUnitLevelEntry(0, 10),
            makeUnitLevelEntry(5, 15),
            makeUnitLevelEntry(10, 20),
          ],
        },
      },
      [
        makeQuestAcceptedEvent(3, 96659),  // player level 10 at t=3
        makeQuestAcceptedEvent(7, 96659),  // player level 15 at t=7
        makeQuestAcceptedEvent(12, 12345), // player level 20 at t=12
      ]
    );

    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 10, // minimum of 10 and 15
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
      {
        entityId: 12345,
        value: 20,
        confidence: "medium",
        provenance: { session: "test-session", t: 12 },
      },
    ]);
  });

  it("should ignore QUEST_ACCEPTED events without matching UnitLevel data", () => {
    const session = makeSession(
      {
        // No UnitLevel stream
      },
      [
        makeQuestAcceptedEvent(3, 96659),
      ]
    );

    expect(observeRequiredLevel(session)).toEqual([]);
  });

  it("should prefer QuestAcceptLevel over fallback when both exist", () => {
    const session = makeSession(
      {
        QuestAcceptLevel: {
          "96659": [{ t: 3, tp: 3, v: 55 }], // direct observation: level 55
        },
        UnitLevel: {
          player: [
            makeUnitLevelEntry(0, 10),
            makeUnitLevelEntry(5, 15),
          ],
        },
      },
      [
        makeQuestAcceptedEvent(3, 96659), // would give level 10 via fallback
      ]
    );

    // Should use QuestAcceptLevel (55), not fallback (10)
    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 55,
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });
});

describe("mergeRequiredLevel", () => {
  it("should return minimum value across observations from multiple sessions", () => {
    const observations = [
      { entityId: 96659, value: 30, confidence: "medium" as const, provenance: { session: "s1", t: 3 } },
      { entityId: 96659, value: 25, confidence: "medium" as const, provenance: { session: "s2", t: 5 } },
      { entityId: 96659, value: 28, confidence: "medium" as const, provenance: { session: "s3", t: 7 } },
    ];

    expect(mergeRequiredLevel(observations)).toBe(25);
  });

  it("should handle single observation", () => {
    const observations = [
      { entityId: 96659, value: 42, confidence: "high" as const, provenance: { session: "s1", t: 3 } },
    ];

    expect(mergeRequiredLevel(observations)).toBe(42);
  });

  it("should handle observations with different confidences", () => {
    const observations = [
      { entityId: 96659, value: 35, confidence: "high" as const, provenance: { session: "s1", t: 3 } },
      { entityId: 96659, value: 25, confidence: "medium" as const, provenance: { session: "s2", t: 5 } },
      { entityId: 96659, value: 30, confidence: "low" as const, provenance: { session: "s3", t: 7 } },
    ];

    // Minimum regardless of confidence
    expect(mergeRequiredLevel(observations)).toBe(25);
  });
});