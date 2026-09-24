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

describe("observeQuestStarts (npc)", () => {
  it("should map a quest to the npcID that started it via questnpc GUID", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 12345, n: 1 } },
      ]
    );

    expect(observeQuestStarts(session)).toEqual([
      {
        entityId: 197,
        value: 96659,
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should map a quest to the npcID that started it via npc GUID", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_ACCEPTED", a: { 1: 96659, n: 1 } },
      ]
    );

    expect(observeQuestStarts(session)).toEqual([
      {
        entityId: 197,
        value: 96659,
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should read the quest ID from argument 2 for the two-argument QUEST_ACCEPTED form", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 11111 }],
        UnitGUID: {
          npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [{ t: 3, tp: 3, e: "QUEST_ACCEPTED", a: { 1: 11111, 2: 96659, n: 2 } }]
    );

    expect(observeQuestStarts(session)).toEqual([
      {
        entityId: 197,
        value: 96659,
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should ignore QUEST_ACCEPTED with a zero quest ID", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [{ t: 3, tp: 3, e: "QUEST_ACCEPTED", a: { 1: 0, n: 1 } }]
    );

    expect(observeQuestStarts(session)).toEqual([]);
  });

  it("should handle multiple quests started by the same NPC", () => {
    const session = makeSession(
      {
        GetQuestID: [
          { t: 3, tp: 3, v: 96659 },
          { t: 10, tp: 10, v: 12345 },
        ],
        UnitGUID: {
          questnpc: [
            { t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" },
            { t: 10, tp: 10, v: "Creature-0-5208-0-7-197-000032" },
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
        entityId: 197,
        value: 96659,
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
      {
        entityId: 197,
        value: 12345,
        confidence: "medium",
        provenance: { session: "test-session", t: 10 },
      },
    ]);
  });

  it("should return empty when no quest-start events exist", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      UnitGUID: {
        questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
      },
    });

    expect(observeQuestStarts(session)).toEqual([]);
  });

  it("should return empty when no npc GUID is active at event time", () => {
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

  it("should return empty when GUID is a player, not an npc", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Player-5284-0362" }],
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
      { entityId: 197, value: 96659, confidence: "medium" as const, provenance: { session: "s", t: 3 } },
      { entityId: 197, value: 96659, confidence: "medium" as const, provenance: { session: "s", t: 10 } },
      { entityId: 197, value: 12345, confidence: "medium" as const, provenance: { session: "s", t: 15 } },
    ];

    expect(mergeQuestStarts(obs)).toEqual(expect.arrayContaining([96659, 12345]));
    expect(mergeQuestStarts(obs)).toHaveLength(2);
  });
});
