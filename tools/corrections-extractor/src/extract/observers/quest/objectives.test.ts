import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { objectivesRealTraceFixture, JUVENILE_VULDREN_CREATURE_ID, SLAY_VULDREN_QUEST_ID } from "./__fixtures__/objectives.realTrace.fixture";
import { observeObjectives } from "./objectives";

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

  it("should match an item objective to a required item of the quest's turn-in window", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "92109": [{ t: 3, tp: 3, v: [{ type: "item", text: "0/5 Empty Vial" }] }],
      },
      GetQuestID: [{ t: 9, tp: 9, v: 92109 }],
      GetQuestItemLink: {
        required: { "1": [{ t: 9, tp: 9, v: "|cnIQ1:|Hitem:3371::::::::10:1485::::::::::|h[Empty Vial]|h|r" }] },
      },
    });

    expect(observeObjectives(session)[0]?.value).toEqual([
      { kind: "item", name: "Empty Vial", text: "0/5 Empty Vial", id: 3371, index: 0 },
    ]);
  });

  it("should match an item objective to an item a merchant sells", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "92109": [{ t: 3, tp: 3, v: [{ type: "item", text: "0/5 Empty Vial" }] }],
      },
      GetMerchantItemLink: {
        "4": [{ t: 5, tp: 5, v: "|cnIQ1:|Hitem:3371::::::::10:1485::::::::::|h[Empty Vial]|h|r" }],
      },
    });

    expect(observeObjectives(session)[0]?.value[0]?.id).toEqual(3371);
  });

  it("should prefer a required item of the quest over a same-named looted item", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "item", text: "Tough Wolf Meat: 0/8" }] }],
      },
      GetLootSlotLink: { "1": [{ t: 4, tp: 4, v: itemLink }] },
      GetQuestID: [{ t: 9, tp: 9, v: 33 }],
      GetQuestItemLink: {
        required: { "1": [{ t: 9, tp: 9, v: "|Hitem:9999::::::::1::::::::::|h[Tough Wolf Meat]|h|r" }] },
      },
    });

    expect(observeObjectives(session)[0]?.value[0]?.id).toEqual(9999);
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

  it("should match a kill objective that ends in another past-tense verb than 'slain'", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "92465": [{ t: 3, tp: 3, v: [{ type: "monster", text: "0/6 Roiling Winds destroyed" }] }],
      },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-251143-000123" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Roiling Winds", n: 1 } }] },
    });

    expect(observeObjectives(session)[0]?.value).toEqual([
      { kind: "monster", name: "Roiling Winds", text: "0/6 Roiling Winds destroyed", id: 251143, index: 0 },
    ]);
  });

  it("should prefer the full objective name over one without its trailing verb", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "object", text: "0/1 Ancient Seed Planted" }] }],
      },
      UnitGUID: {
        target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-100-0000399" }],
        npc: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-200-0000399" }],
      },
      UnitName: {
        target: [{ t: 3, tp: 3, v: { 1: "Ancient Seed", n: 1 } }],
        npc: [{ t: 3, tp: 3, v: { 1: "Ancient Seed Planted", n: 1 } }],
      },
    });

    expect(observeObjectives(session)[0]?.value[0]?.id).toEqual(200);
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
