import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeStartedBy, mergeStartedBy } from "./startedBy";

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

describe("observeStartedBy", () => {
  it("should capture a creature starter from questnpc GUID at QUEST_DETAIL time", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 0, n: 1 } },
      ]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture a creature starter from npc GUID when questnpc is absent", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 0, n: 1 } },
      ]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture an item starter from QUEST_DETAIL questStartItemID", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 750, n: 1 } },
      ]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { itemId: 750 },
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture both creature and item starters for the same quest", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 750, n: 1 } },
      ]
    );

    const results = observeStartedBy(session);
    expect(results).toHaveLength(2);
    expect(results.map((r) => r.value)).toContainEqual({ creatureId: 197 });
    expect(results.map((r) => r.value)).toContainEqual({ itemId: 750 });
  });

  it("should read the quest ID from the one-argument QUEST_ACCEPTED form", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [{ t: 3, tp: 3, e: "QUEST_ACCEPTED", a: { 1: 96659, n: 1 } }]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
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
          questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [{ t: 3, tp: 3, e: "QUEST_ACCEPTED", a: { 1: 11111, 2: 96659, n: 2 } }]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it.each([{ 1: 0, n: 1 }, { 1: "96659", n: 1 }, { n: 0 }])(
    "should ignore invalid QUEST_ACCEPTED payload %#",
    (args) => {
      const session = makeSession(
        {
          GetQuestID: [{ t: 3, tp: 3, v: 11111 }],
          UnitGUID: {
            questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
          },
        },
        [{ t: 3, tp: 3, e: "QUEST_ACCEPTED", a: args }]
      );

      expect(observeStartedBy(session)).toEqual([]);
    }
  );

  it("should capture multiple creature starters across different quest events", () => {
    const session = makeSession(
      {
        GetQuestID: [
          { t: 3, tp: 3, v: 96659 },
          { t: 10, tp: 10, v: 12345 },
        ],
        UnitGUID: {
          questnpc: [
            { t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" },
            { t: 10, tp: 10, v: "Creature-0-5208-0-7-200-000040" },
          ],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 12345, n: 1 } },
        { t: 10, tp: 10, e: "QUEST_DETAIL", a: { 1: 67890, n: 1 } },
      ]
    );

    const results = observeStartedBy(session);
    expect(results).toHaveLength(4);
    // Each quest event produces both a creature observation and an item observation
    expect(results.filter((r) => r.value.creatureId).map((r) => r.entityId)).toEqual([96659, 12345]);
    expect(results.filter((r) => r.value.creatureId).map((r) => r.value.creatureId)).toEqual([197, 200]);
    expect(results.filter((r) => r.value.itemId).map((r) => r.value.itemId)).toEqual([12345, 67890]);
  });

  it("should capture an object starter from GameObject GUID at quest event time", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 0, n: 1 } },
      ]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { objectId: 2843 },
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should ignore player GUIDs seen on quest tokens", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Player-5284-0362" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 0, n: 1 } },
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should return no observations when no quest-start events exist", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      UnitGUID: {
        questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
      },
    });

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should return no observations when GetQuestID is not active at event time", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_DETAIL", a: { 1: 12345, n: 1 } },
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should merge duplicate creature starters via mergeStartedBy", () => {
    const obs = [
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 3 } },
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 10 } },
      { entityId: 96659, value: { creatureId: 200 }, confidence: "medium" as const, provenance: { session: "s", t: 15 } },
    ];

    const result = mergeStartedBy(obs);
    expect(result.creatures).toEqual(expect.arrayContaining([197, 200]));
    expect(result.creatures).toHaveLength(2);
    expect(result.objects).toEqual([]);
    expect(result.items).toEqual([]);
  });

  it("should merge mixed starter types via mergeStartedBy", () => {
    const obs = [
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 3 } },
      { entityId: 96659, value: { objectId: 2843 }, confidence: "low" as const, provenance: { session: "s", t: 5 } },
      { entityId: 96659, value: { itemId: 750 }, confidence: "high" as const, provenance: { session: "s", t: 7 } },
    ];

    const result = mergeStartedBy(obs);
    expect(result.creatures).toEqual([197]);
    expect(result.objects).toEqual([2843]);
    expect(result.items).toEqual([750]);
  });
});
