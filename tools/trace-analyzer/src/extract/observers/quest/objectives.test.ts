import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { objectivesRealTraceFixture, JUVENILE_VULDREN_CREATURE_ID, SLAY_VULDREN_QUEST_ID } from "./__fixtures__/objectives.realTrace.fixture";
import {
  CREATURE_OBJECTIVE_INDEX,
  ITEM_OBJECTIVE_INDEX,
  OBJECT_OBJECTIVE_INDEX,
  mergeQuestObjectives,
  observeObjectives,
  type ObjectiveMatch,
} from "./objectives";

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

const itemLink = "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r";

describe("observeObjectives", () => {
  it("should match an item objective (real suffix-progress format) to a quest-item link and slot information", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "item", text: "Tough Wolf Meat: 0/8" }] }],
      },
      GetLootSlotLink: { "1": [{ t: 4, tp: 4, v: itemLink }] },
      GetLootSlotInfo: {
        "1": [{ t: 4, tp: 4, v: { 2: "Tough Wolf Meat", 6: true, 7: 33, n: 8 } }],
      },
    });

    expect(observeObjectives(session)).toEqual([
      {
        entityId: 33,
        value: [{ kind: "item", name: "Tough Wolf Meat", text: "Tough Wolf Meat: 0/8", id: 750 }],
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should match monster and object objectives (real prefix-progress + kill-suffix format) to UnitName plus UnitGUID", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      "C_QuestLog.GetQuestObjectives": {
        "33": [
          {
            t: 3,
            tp: 3,
            v: [
              { type: "monster", text: "0/10 Kobold Vermin slain" },
              { type: "object", text: "0/1 Suspicious Cache" },
            ],
          },
        ],
      },
      UnitGUID: {
        npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-299-000123" }],
        target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }],
      },
      UnitName: {
        npc: [{ t: 3, tp: 3, v: { 1: "Kobold Vermin", n: 1 } }],
        target: [{ t: 3, tp: 3, v: { 1: "Suspicious Cache", n: 1 } }],
      },
    });

    const matches = observeObjectives(session)[0]?.value;
    expect(matches).toEqual([
      { kind: "monster", name: "Kobold Vermin", text: "0/10 Kobold Vermin slain", id: 299 },
      { kind: "object", name: "Suspicious Cache", text: "0/1 Suspicious Cache", id: 2843 },
    ]);
  });

  it("should not match on singular/plural name variation (exact-name matching only)", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "monster", text: "0/10 Kobold Workers slain" }] }],
      },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-299-000123" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Kobold Worker", n: 1 } }] },
    });

    expect(observeObjectives(session)).toEqual([]);
  });

  it("should match evidence observed anywhere in the session, not only near the objective sample time", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "monster", text: "0/1 Kobold Vermin slain" }] }],
      },
      // The kill happens much later than the objective sample - evidence
      // matching must not be restricted to a time window around t=3.
      UnitGUID: { target: [{ t: 250, tp: 250, v: "Creature-0-5208-0-7-299-000123" }] },
      UnitName: { target: [{ t: 250, tp: 250, v: { 1: "Kobold Vermin", n: 1 } }] },
    });

    expect(observeObjectives(session)[0]?.value[0]).toEqual({
      kind: "monster",
      name: "Kobold Vermin",
      text: "0/1 Kobold Vermin slain",
      id: 299,
    });
  });

  it("should support legacy flat loot streams and traces without GetLocale", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "item", text: "Tough Wolf Meat: 0/8" }] }],
      },
      GetLootSlotLink: [{ t: 4, tp: 4, v: itemLink }],
      GetLootSlotInfo: [{ t: 4, tp: 4, v: { 2: "Tough Wolf Meat", 5: true, 6: 33, n: 8 } }],
    });

    expect(observeObjectives(session)[0]?.value[0]?.id).toEqual(750);
  });

  it("should skip non-enUS objective text", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "deDE" }] },
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "item", text: "Wolfsfleisch: 0/8" }] }],
      },
    });

    expect(observeObjectives(session)).toEqual([]);
  });

  it("should not strip a trailing 'slain' from non-monster objective names", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "object", text: "0/1 Bones of the Slain" }] }],
      },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Bones of the Slain", n: 1 } }] },
    });

    expect(observeObjectives(session)[0]?.value[0]).toEqual({
      kind: "object",
      name: "Bones of the Slain",
      text: "0/1 Bones of the Slain",
      id: 2843,
    });
  });

  it("should skip objective types that never carry an entity id (log/event/empty)", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [
          {
            t: 3,
            tp: 3,
            v: [
              { type: "log", text: "Some flavor text" },
              { type: "event", text: "0/1 Something happened" },
              { type: "", text: "0/1 Untyped" },
            ],
          },
        ],
      },
    });

    expect(observeObjectives(session)).toEqual([]);
  });

  it("should match a real captured objective end-to-end (progress advances, evidence observed later)", () => {
    const observations = observeObjectives(objectivesRealTraceFixture);

    // Both the 0/8 and 1/8 objective samples resolve to the same creature id -
    // the observer does not need the objective and evidence timestamps to align.
    expect(observations).toHaveLength(2);
    for (const observation of observations) {
      expect(observation.entityId).toBe(SLAY_VULDREN_QUEST_ID);
      expect(observation.value).toEqual([
        {
          kind: "monster",
          name: "Juvenile Vuldren",
          text: expect.stringContaining("Juvenile Vuldren slain"),
          id: JUVENILE_VULDREN_CREATURE_ID,
        },
      ]);
    }
  });
});

describe("mergeQuestObjectives", () => {
  it("should emit a positional table with only the buckets that have data (no leading nils)", () => {
    const matches: ObjectiveMatch[] = [
      { kind: "object", name: "Suspicious Cache", text: "0/1 Suspicious Cache", id: 2843 },
      { kind: "item", name: "Tough Wolf Meat", text: "Tough Wolf Meat: 0/8", id: 750 },
    ];

    expect(mergeQuestObjectives([{ entityId: 33, value: matches, confidence: "low", provenance: { session: "s", t: 1 } }])).toEqual({
      [OBJECT_OBJECTIVE_INDEX]: [2843],
      [ITEM_OBJECTIVE_INDEX]: [750],
    });
  });

  it("should dedupe ids for the same objective seen across multiple observations", () => {
    const matches: ObjectiveMatch[] = [
      { kind: "monster", name: "Kobold Vermin", text: "0/10 Kobold Vermin slain", id: 299 },
    ];

    const observations = [
      { entityId: 33, value: matches, confidence: "low" as const, provenance: { session: "s", t: 1 } },
      { entityId: 33, value: matches, confidence: "low" as const, provenance: { session: "s", t: 2 } },
    ];

    expect(mergeQuestObjectives(observations)).toEqual({ [CREATURE_OBJECTIVE_INDEX]: [299] });
  });

  it("should return an empty object when there are no observations", () => {
    expect(mergeQuestObjectives([])).toEqual({});
  });
});
