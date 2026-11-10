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
  it("should take the lowest player level at QUEST_ACCEPTED for each quest", () => {
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

  it("should read the quest ID, not the quest log index, from an Era/TBC QUEST_ACCEPTED", () => {
    const session = makeSession(
      { UnitLevel: { player: [makeUnitLevelEntry(0, 12)] } },
      [{ t: 3, tp: 3, e: "QUEST_ACCEPTED", a: { 1: 3, 2: 96659, n: 2 } }]
    );

    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 96659,
        value: 12,
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should ignore the QuestAcceptLevel stream of traces before schema 18", () => {
    // A real Forever pattern: accepting quest 14 recorded quest 237, the quest at log index 14.
    const session = makeSession(
      {
        QuestAcceptLevel: { "237": [{ t: 3, tp: 3, v: 21 }] },
        UnitLevel: { player: [makeUnitLevelEntry(0, 21)] },
      },
      [makeQuestAcceptedEvent(3, 14)]
    );

    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 14,
        value: 21,
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should use the PLAYER_LEVEL_UP level while the UnitLevel stream still shows the old level", () => {
    // Turning in a quest dings level 10; the follow-up is accepted before the stream catches up.
    const session = makeSession(
      { UnitLevel: { player: [makeUnitLevelEntry(0, 9), makeUnitLevelEntry(51, 10)] } },
      [
        { t: 50, tp: 50, e: "PLAYER_LEVEL_UP", a: { 1: 10, n: 1 } },
        makeQuestAcceptedEvent(50.2, 5001),
      ]
    );

    expect(observeRequiredLevel(session)).toEqual([
      {
        entityId: 5001,
        value: 10,
        confidence: "medium",
        provenance: { session: "test-session", t: 50.2 },
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