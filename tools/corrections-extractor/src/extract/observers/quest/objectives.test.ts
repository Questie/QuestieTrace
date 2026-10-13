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
        "1": [{ t: 4, tp: 4, v: { 2: "Tough Wolf Meat", 7: true, n: 10 } }],
      },
    });

    expect(observeObjectives(session)).toEqual([
      {
        entityId: 33,
        value: [{ kind: "item", name: "Tough Wolf Meat", text: "Tough Wolf Meat: 0/8", id: 750, index: 0 }],
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should pair keyed loot streams by slot and leave missing info streams empty", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "item", text: "Tough Wolf Meat: 0/8" }] }],
      },
      GetLootSlotLink: {
        "1": [{ t: 4, tp: 4, v: itemLink }],
        "2": [{ t: 5, tp: 5, v: itemLink }],
      },
      GetLootSlotInfo: {
        "2": [{ t: 5, tp: 5, v: { 2: "Tough Wolf Meat", 7: true, n: 10 } }],
      },
    });

    expect(observeObjectives(session)[0]?.value[0]?.id).toEqual(750);
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
      { kind: "monster", name: "Kobold Vermin", text: "0/10 Kobold Vermin slain", id: 299, index: 0 },
      { kind: "object", name: "Suspicious Cache", text: "0/1 Suspicious Cache", id: 2843, index: 1 },
    ]);
  });

  it("should keep the API index of a matched objective that follows an unmatched one", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [
          {
            t: 3,
            tp: 3,
            v: [
              { type: "item", text: "Twisted Water Totem: 0/5" },
              { type: "item", text: "Tough Wolf Meat: 0/8" },
            ],
          },
        ],
      },
      GetLootSlotLink: { "1": [{ t: 4, tp: 4, v: itemLink }] },
    });

    expect(observeObjectives(session)[0]?.value).toEqual([
      { kind: "item", name: "Tough Wolf Meat", text: "Tough Wolf Meat: 0/8", id: 750, index: 1 },
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
      index: 0,
    });
  });

  it("should match unit evidence whose GetQuestID attribution differs from the quest (weak signal, tie-break only)", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "monster", text: "0/1 Kobold Vermin slain" }] }],
      },
      // GetQuestID reported some other quest at the kill's timestamp; the
      // objective name still matches exactly, so the match is accepted.
      GetQuestID: [{ t: 3, tp: 3, v: 999 }],
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-299-000123" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Kobold Vermin", n: 1 } }] },
    });

    expect(observeObjectives(session)[0]?.value[0]).toEqual({
      kind: "monster",
      name: "Kobold Vermin",
      text: "0/1 Kobold Vermin slain",
      id: 299,
      index: 0,
    });
  });

  it("should prefer same-quest unit evidence over cross-quest evidence, breaking remaining ties by lowest id", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "monster", text: "0/1 Kobold Vermin slain" }] }],
      },
      GetQuestID: [
        { t: 3, tp: 3, v: 33 },
        { t: 5, tp: 5, v: 999 },
      ],
      // Both units match the objective name; the one attributed to quest 33
      // (id 299) must win over the cross-quest one (id 300).
      UnitGUID: {
        target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-299-000123" }],
        npc: [{ t: 5, tp: 5, v: "Creature-0-5208-0-7-300-000456" }],
      },
      UnitName: {
        target: [{ t: 3, tp: 3, v: { 1: "Kobold Vermin", n: 1 } }],
        npc: [{ t: 5, tp: 5, v: { 1: "Kobold Vermin", n: 1 } }],
      },
    });

    expect(observeObjectives(session)[0]?.value[0]?.id).toEqual(299);
  });

  it("should fall back to an item that starts the quest when no other item matches", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "item", text: "Tough Wolf Meat: 0/8" }] }],
      },
      GetLootSlotLink: { "1": [{ t: 4, tp: 4, v: itemLink }] },
      GetLootSlotInfo: {
        // GetLootSlotInfo: 7 = isQuestItem, 8 = questID of the quest the item starts, 9 = isActive.
        "1": [{ t: 4, tp: 4, v: { 2: "Tough Wolf Meat", 7: false, 8: 33, 9: false, n: 10 } }],
      },
    });

    expect(observeObjectives(session)[0]?.value[0]?.id).toEqual(750);
  });

  it("should prefer any other same-named item over one that starts the quest", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "86613": [{ t: 3, tp: 3, v: [{ type: "item", text: "0/6 Excavation Tools" }] }],
      },
      GetLootSlotLink: {
        "1": [
          { t: 1, tp: 1, v: "|cnIQ1:|Hitem:278049::::::::15:1487:::::::::|h[Excavation Tools]|h|r" },
          { t: 5, tp: 5, v: "|cnIQ1:|Hitem:278398::::::::15:1487:::::::::|h[Excavation Tools]|h|r" },
        ],
      },
      GetLootSlotInfo: {
        "1": [
          { t: 1, tp: 1, v: { 2: "Excavation Tools", 7: false, 8: 86613, 9: false, n: 10 } },
          { t: 5, tp: 5, v: { 2: "Excavation Tools", 7: false, n: 10 } },
        ],
      },
    });

    expect(observeObjectives(session)[0]?.value[0]?.id).toEqual(278398);
  });

  it("should prefer a quest item over a same-named quest starter item with a lower id", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "86613": [{ t: 3, tp: 3, v: [{ type: "item", text: "0/6 Excavation Tools" }] }],
      },
      GetLootSlotLink: {
        "1": [
          { t: 1, tp: 1, v: "|cnIQ1:|Hitem:278049::::::::15:1487:::::::::|h[Excavation Tools]|h|r" },
          { t: 5, tp: 5, v: "|cnIQ1:|Hitem:278398::::::::15:1487:::::::::|h[Excavation Tools]|h|r" },
        ],
      },
      GetLootSlotInfo: {
        "1": [
          { t: 1, tp: 1, v: { 2: "Excavation Tools", 7: false, 8: 86000, 9: false, n: 10 } },
          { t: 5, tp: 5, v: { 2: "Excavation Tools", 7: true, n: 10 } },
        ],
      },
    });

    expect(observeObjectives(session)[0]?.value[0]?.id).toEqual(278398);
  });

  it("should support legacy flat loot streams and traces without GetLocale", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "item", text: "Tough Wolf Meat: 0/8" }] }],
      },
      GetLootSlotLink: [{ t: 4, tp: 4, v: itemLink }],
      GetLootSlotInfo: [{ t: 4, tp: 4, v: { 2: "Tough Wolf Meat", 7: true, n: 10 } }],
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
      index: 0,
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
          index: 0,
        },
      ]);
    }
  });
});

describe("mergeQuestObjectives", () => {
  function obs(value: ObjectiveMatch[], t = 1, session = "s") {
    return { entityId: 33, value, confidence: "low" as const, provenance: { session, t } };
  }
  function item(id: number, index: number): ObjectiveMatch {
    return { kind: "item", name: `Item ${id}`, text: `Item ${id}: 0/1`, id, index };
  }

  it("should emit a positional table with only the buckets that have data (no leading nils)", () => {
    const matches: ObjectiveMatch[] = [
      { kind: "object", name: "Suspicious Cache", text: "0/1 Suspicious Cache", id: 2843, index: 0 },
      { kind: "item", name: "Tough Wolf Meat", text: "Tough Wolf Meat: 0/8", id: 750, index: 1 },
    ];

    expect(mergeQuestObjectives([obs(matches)])).toEqual({
      [OBJECT_OBJECTIVE_INDEX]: [2843],
      [ITEM_OBJECTIVE_INDEX]: [750],
    });
  });

  it("should dedupe ids for the same objective seen across multiple observations", () => {
    const matches: ObjectiveMatch[] = [
      { kind: "monster", name: "Kobold Vermin", text: "0/10 Kobold Vermin slain", id: 299, index: 0 },
    ];

    expect(mergeQuestObjectives([obs(matches, 1), obs(matches, 2)])).toEqual({ [CREATURE_OBJECTIVE_INDEX]: [299] });
  });

  it("should emit one id per objective when observations resolve the same objective to different ids", () => {
    const hecklefang = (id: number): ObjectiveMatch => ({
      kind: "monster", name: "Hecklefang Stalker", text: "0/7 Hecklefang Stalker slain", id, index: 0,
    });
    const stormhide = (id: number): ObjectiveMatch => ({
      kind: "monster", name: "Stormhide", text: "0/7 Stormhide slain", id, index: 1,
    });

    const result = mergeQuestObjectives([
      obs([hecklefang(4128), stormhide(3456)], 1, "a"),
      obs([hecklefang(4128), stormhide(3238)], 2, "b"),
      obs([stormhide(3238)], 3, "c"),
    ]);

    expect(result).toEqual({ [CREATURE_OBJECTIVE_INDEX]: [4128, 3238] });
  });

  it("should prefer the lowest id when conflicting ids for an objective are seen in equally many sessions", () => {
    const result = mergeQuestObjectives([obs([item(20, 0)], 1, "a"), obs([item(10, 0)], 2, "b")]);

    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([10]);
  });

  it("should count repeated observations from one session only once", () => {
    const result = mergeQuestObjectives([
      obs([item(10, 0)], 1, "a"),
      obs([item(10, 0)], 2, "a"),
      obs([item(10, 0)], 3, "a"),
      obs([item(20, 0)], 4, "b"),
      obs([item(20, 0)], 5, "c"),
    ]);

    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([20]);
  });

  it("should return an empty object when there are no observations", () => {
    expect(mergeQuestObjectives([])).toEqual({});
  });

  it("should order ids by their API objective index, not numerically", () => {
    const result = mergeQuestObjectives([obs([item(251291, 0), item(251284, 1), item(251261, 2)])]);

    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([251291, 251284, 251261]);
  });

  it("should keep API order when observations only matched some objectives", () => {
    // Water(0), Earth(1), Fire(2), Air(3): evidence for each totem shows up at different times.
    const result = mergeQuestObjectives([
      obs([item(274046, 2)], 1),
      obs([item(274044, 1), item(274046, 2), item(274047, 3)], 2),
      obs([item(274045, 0), item(274044, 1), item(274046, 2), item(274047, 3)], 3, "other-session"),
    ]);

    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([274045, 274044, 274046, 274047]);
  });

  it("should keep API order for mixed objective types independently", () => {
    const matches: ObjectiveMatch[] = [
      { kind: "monster", name: "Mob B", text: "0/1 Mob B slain", id: 200, index: 0 },
      { kind: "object", name: "Obj A", text: "0/1 Obj A", id: 100, index: 1 },
      { kind: "monster", name: "Mob A", text: "0/1 Mob A slain", id: 100, index: 2 },
      { kind: "item", name: "Item B", text: "Item B: 0/1", id: 300, index: 3 },
      { kind: "object", name: "Obj B", text: "0/1 Obj B", id: 200, index: 4 },
      { kind: "item", name: "Item A", text: "Item A: 0/1", id: 100, index: 5 },
    ];

    const result = mergeQuestObjectives([obs(matches)]);

    expect(result[CREATURE_OBJECTIVE_INDEX]).toEqual([200, 100]);
    expect(result[OBJECT_OBJECTIVE_INDEX]).toEqual([100, 200]);
    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([300, 100]);
  });
});
