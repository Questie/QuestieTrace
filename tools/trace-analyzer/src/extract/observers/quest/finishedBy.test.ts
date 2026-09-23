import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeFinishedBy, mergeFinishedBy } from "./finishedBy";

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

describe("observeFinishedBy", () => {
  it("should capture a creature finisher from questnpc GUID at QUEST_COMPLETE time", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_COMPLETE", a: { n: 0 } },
      ]
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture a creature finisher from npc GUID when questnpc is absent", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_COMPLETE", a: { n: 0 } },
      ]
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture an object finisher from GameObject GUID at quest completion time", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_COMPLETE", a: { n: 0 } },
      ]
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { objectId: 2843 },
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture both creature and object finishers for the same quest", () => {
    const session = makeSession(
      {
        GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
          target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_COMPLETE", a: { n: 0 } },
      ]
    );

    const results = observeFinishedBy(session);
    expect(results).toHaveLength(2);
    expect(results.map((r) => r.value)).toContainEqual({ creatureId: 197 });
    expect(results.map((r) => r.value)).toContainEqual({ objectId: 2843 });
  });

  it("should capture multiple creature finishers across different quest events", () => {
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
        { t: 3, tp: 3, e: "QUEST_COMPLETE", a: { n: 0 } },
        { t: 10, tp: 10, e: "QUEST_COMPLETE", a: { n: 0 } },
      ]
    );

    const results = observeFinishedBy(session);
    expect(results).toHaveLength(2);
    expect(results.filter((r) => r.value.creatureId).map((r) => r.entityId)).toEqual([96659, 12345]);
    expect(results.filter((r) => r.value.creatureId).map((r) => r.value.creatureId)).toEqual([197, 200]);
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
        { t: 3, tp: 3, e: "QUEST_COMPLETE", a: { n: 0 } },
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should return no observations when no quest-completion events exist", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      UnitGUID: {
        questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
      },
    });

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should return no observations when GetQuestID is not active at event time", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-197-000032" }],
        },
      },
      [
        { t: 3, tp: 3, e: "QUEST_COMPLETE", a: { n: 0 } },
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should merge duplicate creature finishers via mergeFinishedBy", () => {
    const obs = [
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 3 } },
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 10 } },
      { entityId: 96659, value: { creatureId: 200 }, confidence: "medium" as const, provenance: { session: "s", t: 15 } },
    ];

    const result = mergeFinishedBy(obs);
    expect(result.creatures).toEqual(expect.arrayContaining([197, 200]));
    expect(result.creatures).toHaveLength(2);
    expect(result.objects).toEqual([]);
  });

  it("should merge mixed finisher types via mergeFinishedBy", () => {
    const obs = [
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 3 } },
      { entityId: 96659, value: { objectId: 2843 }, confidence: "low" as const, provenance: { session: "s", t: 5 } },
    ];

    const result = mergeFinishedBy(obs);
    expect(result.creatures).toEqual([197]);
    expect(result.objects).toEqual([2843]);
  });
});
