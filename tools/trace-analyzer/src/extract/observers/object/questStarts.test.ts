import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeQuestStarts, mergeQuestStarts } from "./questStarts";

function makeSession(functions: SessionRecord["functions"], events: SessionRecord["events"] = []): SessionRecord {
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

describe("observeQuestStarts (object)", () => {
  it("should map a quest to the objectID that started it via target GUID", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 12345, n: 1 } },
      ]
    );

    expect(observeQuestStarts(session)).toEqual([
      {
        entityId: 2843,
        value: 96659,
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should map a quest to the objectID via npc token GUID", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          npc: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_ACCEPTED", a: { 1: 96659, n: 1 } },
      ]
    );

    expect(observeQuestStarts(session)).toEqual([
      {
        entityId: 2843,
        value: 96659,
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should read the quest ID from argument 2 for the two-argument QUEST_ACCEPTED form", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 11111 }],
        UnitGUID: {
          npc: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }],
        },
      },
      [{ t: 3, tp: 3, e: "QUEST_ACCEPTED", a: { 1: 11111, 2: 96659, n: 2 } }]
    );

    expect(observeQuestStarts(session)).toEqual([
      {
        entityId: 2843,
        value: 96659,
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should ignore QUEST_ACCEPTED with a zero quest ID", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }],
        },
      },
      [{ t: 3, tp: 3, e: "QUEST_ACCEPTED", a: { 1: 0, n: 1 } }]
    );

    expect(observeQuestStarts(session)).toEqual([]);
  });

  it("should handle multiple quests started by the same object", () => {
    const session = makeSession(
      {
        GetQuestID: [
          { t: 3, tp: 3, v: 96659 },
          { t: 10, tp: 10, v: 12345 },
        ],
        UnitGUID: {
          target: [
            { t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" },
            { t: 10, tp: 10, v: "GameObject-0-5208-0-7-2843-0000399" },
          ],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 111, n: 1 } },
        { t: 10, tp: 10, e: "QUEST_DETAIL", a: { 1: 222, n: 1 } },
      ]
    );

    expect(observeQuestStarts(session)).toEqual([
      {
        entityId: 2843,
        value: 96659,
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
      },
      {
        entityId: 2843,
        value: 12345,
        confidence: "low",
        provenance: { session: "test-session", t: 10 },
      },
    ]);
  });

  it("should return empty when no object GUID is active at event time", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 12345, n: 1 } },
      ]
    );

    expect(observeQuestStarts(session)).toEqual([]);
  });

  it("should return empty when GUID is a creature, not an object", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 12345, n: 1 } },
      ]
    );

    expect(observeQuestStarts(session)).toEqual([]);
  });

  it("should merge duplicate questIDs via mergeQuestStarts", () => {
    const obs = [
      { entityId: 2843, value: 96659, confidence: "low" as const, provenance: { session: "s", t: 3 } },
      { entityId: 2843, value: 96659, confidence: "low" as const, provenance: { session: "s", t: 10 } },
      { entityId: 2843, value: 12345, confidence: "low" as const, provenance: { session: "s", t: 15 } },
    ];

    expect(mergeQuestStarts(obs)).toEqual(expect.arrayContaining([96659, 12345]));
    expect(mergeQuestStarts(obs)).toHaveLength(2);
  });
});
