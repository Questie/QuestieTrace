import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../core/types";
import { extractAll } from "./index";

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

describe("extractAll (full chain: observe -> aggregate -> emit -> write)", () => {
  it("should produce a paste-ready ForeverTraceNpcFixes.lua correction for an npc encountered in the session", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { npc: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 1 } }] },
      "C_Map.GetBestMapForUnit": { player: [{ t: 3, tp: 3, v: 1436 }] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [{ t: 3, tp: 3, v: { x: 0.3001, y: 0.8602 } }] },
    });

    const { npcFixes } = extractAll([session], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
    });

    expect(npcFixes).toContain("function ForeverTraceNpcFixes:Load()");
    expect(npcFixes).toContain("-- Source trace files: 1");
    expect(npcFixes).toContain("[823] = {");
    expect(npcFixes).toContain('[npcKeys.name] = "Deputy Willem",');
    expect(npcFixes).toContain("[npcKeys.spawns] = {[40]={{30.01,86.02}}},");
  });

  it("should only emit fields that were actually observed, not every schema field", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      "C_Map.GetBestMapForUnit": { player: [{ t: 3, tp: 3, v: 1436 }] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [{ t: 3, tp: 3, v: { x: 0.3001, y: 0.8602 } }] },
    });

    const { npcFixes } = extractAll([session], { sourceFileNames: ["test.lua"] });

    // minLevel/maxLevel have no observer yet (see observers/npc/minLevel.ts), so
    // they must not appear in the correction at all - no defaulting to 0/nil.
    expect(npcFixes).not.toContain("npcKeys.minLevel");
    expect(npcFixes).not.toContain("npcKeys.maxLevel");
  });
});

describe("extractAll (quest/item/object entities)", () => {
  it("should produce a paste-ready ForeverTraceQuestFixes.lua correction for a quest encountered in the session", () => {
    const session = makeSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      QuestLogZone: { "96659": [{ t: 2, tp: 2, v: "Westfall" }] },
      "C_QuestLog.GetQuestObjectives": {
        "96659": [
          {
            t: 3,
            tp: 3,
            v: [
              { type: "item", text: "Tough Wolf Meat: 0/8" },
              { type: "event", text: "Light the campfire", finished: true },
            ],
          },
        ],
      },
      GetLootSlotLink: {
        "1": [{ t: 4, tp: 4, v: "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }],
      },
      GetLootSlotInfo: {
        "1": [{ t: 4, tp: 4, v: { 2: "Tough Wolf Meat", 6: true, 7: 96659, n: 8 } }],
      },
      GetTitleText: [{ t: 3, tp: 3, v: "A Threat Within" }],
      GetLocale: { player: [{ t: 3, tp: 3, v: "enUS" }] },
      "C_Map.GetBestMapForUnit": { player: [{ t: 3, tp: 3, v: 1436 }] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [{ t: 3, tp: 3, v: { x: 0.3001, y: 0.8602 } }] },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", 2: "", n: 2 } }] },
    });

    const { questFixes } = extractAll([session], { sourceFileNames: ["test.lua"] });

    expect(questFixes).toContain("function ForeverTraceQuestFixes:Load()");
    expect(questFixes).toContain("[96659] = {");
    expect(questFixes).toContain('[questKeys.name] = "A Threat Within",');
    expect(questFixes).toContain("[questKeys.zoneOrSort] = 40,");
    expect(questFixes).toContain("[questKeys.objectives] = {[3]={750}},");
    expect(questFixes).toContain('[questKeys.triggerEnd] = {"Light the campfire",{[40]={{30.01,86.02}}}},');  });

  it("should produce a paste-ready ForeverTraceItemFixes.lua correction for a looted item", () => {
    const session = makeSession({
      GetLootSlotLink: { "1": [{ t: 0, tp: 0, v: "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }] },
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      "C_Map.GetBestMapForUnit": { player: [{ t: 0, tp: 0, v: 1436 }] },
      "C_Map.GetPlayerMapPosition": { player: [{ t: 0, tp: 0, v: { x: 0.3001, y: 0.8602 } }] },
    });

    const { itemFixes } = extractAll([session], { sourceFileNames: ["test.lua"] });

    expect(itemFixes).toContain("function ForeverTraceItemFixes:Load()");
    expect(itemFixes).toContain("[750] = {");
    expect(itemFixes).toContain('[itemKeys.name] = "Tough Wolf Meat",');
  });

  it("should produce a paste-ready ForeverTraceObjectFixes.lua correction for a GameObject encountered in the session", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Suspicious Chest", n: 1 } }] },
      "C_Map.GetBestMapForUnit": { player: [{ t: 3, tp: 3, v: 1436 }] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [{ t: 3, tp: 3, v: { x: 0.3001, y: 0.8602 } }] },
    });

    const { objectFixes } = extractAll([session], { sourceFileNames: ["test.lua"] });

    expect(objectFixes).toContain("function ForeverTraceObjectFixes:Load()");
    expect(objectFixes).toContain("[2843] = {");
    expect(objectFixes).toContain('[objectKeys.name] = "Suspicious Chest",');
    expect(objectFixes).toContain("[objectKeys.spawns] = {[40]={{30.01,86.02}}},");
  });
});
