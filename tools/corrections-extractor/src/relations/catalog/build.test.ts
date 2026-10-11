import { describe, expect, it } from "vitest";
import { buildGroundTruth, catalogQuestOf, relationSetOf } from "./build";
import type { MaterializedQuests } from "./materialize";

describe("relationSetOf", () => {
  it("turns scalars into one-element lists and drops Questie's empty values", () => {
    expect(
      relationSetOf([{ nextQuestInChain: 5, parentQuest: 0, preQuestSingle: [], exclusiveTo: [7, 0, 8], preQuestGroup: [3, -4] }]),
    ).toEqual({ nextQuestInChain: [5], exclusiveTo: [7, 8], preQuestGroup: [3, 4] });
  });

  it("unions the targets of faction and class views", () => {
    expect(relationSetOf([{ nextQuestInChain: 3370, preQuestSingle: [1] }, { nextQuestInChain: 3369, preQuestSingle: [1] }])).toEqual({
      nextQuestInChain: [3370, 3369],
      preQuestSingle: [1],
    });
  });
});

describe("catalogQuestOf", () => {
  it("unions givers across views and never drops a requirement only some views have", () => {
    const quest = catalogQuestOf(7562, [
      { name: "Mor'zul", questLevel: 58, requiredLevel: 60, requiredRaces: 0, requiredSkill: [0, 0], startedBy: [[5520, 6382], null, [9]], finishedBy: [[14436]] },
      { name: "Mor'zul", questLevel: 58, requiredLevel: 55, requiredRaces: 0, requiredSkill: [186, 100], startedBy: [[5753, 5520]], finishedBy: [[14436]] },
    ]);
    expect(quest).toStrictEqual({
      id: 7562,
      name: "Mor'zul",
      questLevel: 58,
      requiredLevel: 60,
      requiredRaces: undefined,
      requiredClasses: undefined,
      requiredSkill: [186, 100],
      requiredMinRep: undefined,
      requiredMaxRep: undefined,
      requiredSpell: undefined,
      specialFlags: undefined,
      starters: { npcs: [5520, 6382, 5753], objects: [], items: [9] },
      finishers: { npcs: [14436], objects: [] },
    });
  });
});

describe("buildGroundTruth", () => {
  it("tiers quests by provenance: every inherited quest, and Forever-new ones with authored relations", () => {
    const materialized: MaterializedQuests = {
      sources: [],
      quests: {
        "100": {},
        "490": {},
        "65593": { nextQuestInChain: 65594 },
        "9666": { preQuestSingle: [100] },
        "9667": { requiredRaces: 2 },
        "9668": { nextQuestInChain: 9666 },
        "9669": {},
      },
      dynamicViews: {},
      authoredFields: { "490": ["name"], "9666": ["preQuestSingle"], "9667": ["requiredRaces"], "9669": ["exclusiveTo"] },
      inheritedIds: [100, 65593],
    };
    expect(buildGroundTruth(materialized, "").tiers).toEqual({
      // 100: no relations is still known. 65593: inherited from a legacy fix despite its id.
      inheritedClassic: { "100": {}, "65593": { nextQuestInChain: [65594] } },
      // 490: Forever-new despite its id, no authored relation. 9667: authored, but not a relation.
      // 9668: relation from elsewhere. 9669: authored deletion.
      authoredForever: { "9666": { preQuestSingle: [100] }, "9669": {} },
    });
  });
});
