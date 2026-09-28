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

/** Creates a session with a Forever interfaceVersion (16001) */
function makeForeverSession(functions: SessionRecord["functions"]): SessionRecord {
  return makeSession({
    ...functions,
    GetBuildInfo: { player: [{ t: 0, tp: 0, v: 16001 }] },
  });
}

/** Creates a session with a TBC interfaceVersion (10xxx) */
function makeTBCSession(functions: SessionRecord["functions"]): SessionRecord {
  return makeSession({
    ...functions,
    GetBuildInfo: { player: [{ t: 0, tp: 0, v: 10102 }] },
  });
}

describe("extractAll (full chain: observe -> aggregate -> emit -> write)", () => {
  it("should produce a paste-ready foreverNpcTraces.lua correction for an npc encountered in the session", () => {
    const session = makeForeverSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { npc: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 1 } }] },
      "C_Map.GetBestMapForUnit": { player: [{ t: 3, tp: 3, v: 1436 }] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [{ t: 3, tp: 3, v: { x: 0.3001, y: 0.8602 } }] },
    });

    const { npcFixes } = extractAll([session], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    expect(npcFixes).toContain("function ForeverNpcTraces:Load()");
    expect(npcFixes).toContain("-- Source trace files: 1");
    expect(npcFixes).toContain("[823] = {");
    expect(npcFixes).toContain('[npcKeys.name] = "Deputy Willem",');
    expect(npcFixes).toContain("[npcKeys.spawns] = {[40]={{30.01,86.02}}},");
  });

  it("should only emit fields that were actually observed, not every schema field", () => {
    const session = makeForeverSession({
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
  it("should produce a paste-ready foreverQuestTraces.lua correction for a quest encountered in the session", () => {
    const session = makeForeverSession({
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

    const { questFixes } = extractAll([session], { sourceFileNames: ["test.lua"], maxIds: { npc: 0, quest: 0, item: 0, object: 0 } });

    expect(questFixes).toContain("function ForeverQuestTraces:Load()");

    expect(questFixes).toContain("function ForeverQuestTraces:Load()");
    expect(questFixes).toContain("[96659] = {");
    expect(questFixes).toContain('[questKeys.name] = "A Threat Within",');
    expect(questFixes).toContain("[questKeys.zoneOrSort] = 40,");
    expect(questFixes).toContain("[questKeys.objectives] = {nil,nil,{{750}}},");
    expect(questFixes).toContain('[questKeys.triggerEnd] = {"Light the campfire",{[40]={{30.01,86.02}}}},');  });

  it("should export objectivesText as a line list, keeping blank separator lines", () => {
    const session = makeForeverSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      "GetQuestLogQuestText": {
        "25": [
          {
            t: 10,
            tp: 10,
            v: {
              1: "Warlord Senta has a problem...",
              2: "Dispatch 12 Befouled Water Elementals at Mystral Lake.\n\nScout the gazebo on Mystral Lake.\n\nReturn to Mastok Wrilehiss.",
              n: 2,
            },
          },
        ],
      },
    });

    const { questFixes } = extractAll([session], { sourceFileNames: ["test.lua"], maxIds: { npc: 0, quest: 0, item: 0, object: 0 } });

    expect(questFixes).toContain(
      '[questKeys.objectivesText] = {"Dispatch 12 Befouled Water Elementals at Mystral Lake.","","Scout the gazebo on Mystral Lake.","","Return to Mastok Wrilehiss."},',
    );
    expect(questFixes).not.toContain("Warlord Senta has a problem");
  });

  it("should produce a paste-ready foreverItemTraces.lua correction for a looted item", () => {
    const session = makeForeverSession({
      GetLootSlotLink: { "1": [{ t: 0, tp: 0, v: "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }] },
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      "C_Map.GetBestMapForUnit": { player: [{ t: 0, tp: 0, v: 1436 }] },
      "C_Map.GetPlayerMapPosition": { player: [{ t: 0, tp: 0, v: { x: 0.3001, y: 0.8602 } }] },
    });

    const { itemFixes } = extractAll([session], { sourceFileNames: ["test.lua"], maxIds: { npc: 0, quest: 0, item: 0, object: 0 } });

    expect(itemFixes).toContain("function ForeverItemTraces:Load()");
    expect(itemFixes).toContain("[750] = {");
    expect(itemFixes).toContain('[itemKeys.name] = "Tough Wolf Meat",');
  });

  it("should produce a paste-ready foreverObjectTraces.lua correction for a GameObject encountered in the session", () => {
    const session = makeForeverSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Suspicious Chest", n: 1 } }] },
      "C_Map.GetBestMapForUnit": { player: [{ t: 3, tp: 3, v: 1436 }] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [{ t: 3, tp: 3, v: { x: 0.3001, y: 0.8602 } }] },
    });

    const { objectFixes } = extractAll([session], { sourceFileNames: ["test.lua"], maxIds: { npc: 0, quest: 0, item: 0, object: 0 } });

    expect(objectFixes).toContain("function ForeverObjectTraces:Load()");
    expect(objectFixes).toContain("[2843] = {");
    expect(objectFixes).toContain('[objectKeys.name] = "Suspicious Chest",');
    expect(objectFixes).toContain("[objectKeys.spawns] = {[40]={{30.01,86.02}}},");
  });

  it("should export triggerEnd for the quest's zoneOrSort zone with a single coordinate pair", () => {
    // The event objective is completed once per session (each session observes
    // one incomplete->complete transition): in Elwynn Forest (uiMapID 1429 ->
    // areaID 12) and in Westfall (uiMapID 1436 -> areaID 40). The quest log
    // header says Westfall, so the Westfall completion must win.
    const sessionInElwynn = makeForeverSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      QuestLogZone: { "96659": [{ t: 2, tp: 2, v: "Westfall" }] },
      "C_QuestLog.GetQuestObjectives": {
        "96659": [
          { t: 8, tp: 8, v: [{ type: "event", text: "Light the campfire", finished: false }] },
          { t: 12, tp: 12, v: [{ type: "event", text: "Light the campfire", finished: true }] },
        ],
      },
      "C_Map.GetBestMapForUnit": { player: [{ t: 12, tp: 12, v: 1429 }] },
      "C_Map.GetPlayerMapPosition": { player: [{ t: 12, tp: 12, v: { x: 0.4746, y: 0.6218 } }] },
    });
    const sessionInWestfall = makeForeverSession({
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      QuestLogZone: { "96659": [{ t: 2, tp: 2, v: "Westfall" }] },
      "C_QuestLog.GetQuestObjectives": {
        "96659": [
          { t: 16, tp: 16, v: [{ type: "event", text: "Light the campfire", finished: false }] },
          { t: 20, tp: 20, v: [{ type: "event", text: "Light the campfire", finished: true }] },
        ],
      },
      "C_Map.GetBestMapForUnit": { player: [{ t: 20, tp: 20, v: 1436 }] },
      "C_Map.GetPlayerMapPosition": { player: [{ t: 20, tp: 20, v: { x: 0.5611, y: 0.6164 } }] },
    });

    const { questFixes } = extractAll([sessionInElwynn, sessionInWestfall], { sourceFileNames: ["test.lua"], maxIds: { npc: 0, quest: 0, item: 0, object: 0 } });

    expect(questFixes).toContain("[questKeys.zoneOrSort] = 40,");
    expect(questFixes).toContain('[questKeys.triggerEnd] = {"Light the campfire",{[40]={{56.11,61.64}}}},');
  });
});

describe("extractAll interfaceVersion filtering", () => {
  it("should include sessions without GetBuildInfo (early Forever beta traces)", () => {
    // Sessions without GetBuildInfo are included because they might be from
    // early Forever beta when PlayerIdentity.lua didn't capture interfaceVersion yet.
    // Any Classic IDs they contain will be filtered by filterBelowMax.
    const sessionWithoutBuildInfo = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { npc: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 1 } }] },
    });

    const { npcFixes } = extractAll([sessionWithoutBuildInfo], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    // Session without GetBuildInfo should be INCLUDED
    expect(npcFixes).toContain("function ForeverNpcTraces:Load()");
    expect(npcFixes).toContain("[823] = {");
    expect(npcFixes).toContain('[npcKeys.name] = "Deputy Willem",');
  });

  it("should skip sessions with TBC interfaceVersion (10xxx)", () => {
    const tbcSession = makeTBCSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { npc: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 1 } }] },
    });

    const { npcFixes } = extractAll([tbcSession], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    // TBC session should be filtered out
    expect(npcFixes).toContain("function ForeverNpcTraces:Load()");
    expect(npcFixes).toContain("    return {\n    }");
    expect(npcFixes).not.toContain("[823]");
  });

  it("should skip sessions with WotLK interfaceVersion (11xxx)", () => {
    const wotlkSession = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { npc: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 1 } }] },
      GetBuildInfo: { player: [{ t: 0, tp: 0, v: 11883 }] },
    });

    const { npcFixes } = extractAll([wotlkSession], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    // WotLK session should be filtered out
    expect(npcFixes).toContain("function ForeverNpcTraces:Load()");
    expect(npcFixes).toContain("    return {\n    }");
    expect(npcFixes).not.toContain("[823]");
  });

  it("should process sessions with Forever interfaceVersion (16xxx)", () => {
    const foreverSession = makeForeverSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { npc: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 1 } }] },
    });

    const { npcFixes } = extractAll([foreverSession], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    // Forever session should be processed
    expect(npcFixes).toContain("[823] = {");
    expect(npcFixes).toContain('[npcKeys.name] = "Deputy Willem",');
  });

  it("should process mixed sessions, only including Forever builds", () => {
    const foreverSession = makeForeverSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { npc: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 1 } }] },
    });
    const tbcSession = makeTBCSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { npc: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-999999" }] },
      UnitName: { npc: [{ t: 3, tp: 3, v: { 1: "TBC NPC", n: 1 } }] },
    });

    const { npcFixes } = extractAll([tbcSession, foreverSession], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    // Only the Forever session's data should appear (ID 823, not 999999)
    expect(npcFixes).toContain("[823] = {");
    expect(npcFixes).toContain('[npcKeys.name] = "Deputy Willem",');
    expect(npcFixes).not.toContain("[999999]");
    expect(npcFixes).not.toContain("TBC NPC");
  });

  it("should report correct session count for mixed Forever/non-Forever sessions", () => {
    const foreverSession1 = makeForeverSession({ GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] } });
    const foreverSession2 = makeForeverSession({ GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] } });
    const tbcSession = makeTBCSession({ GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] } });

    const { npcFixes } = extractAll([foreverSession1, tbcSession, foreverSession2], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
    });

    // Should report 2 sessions, not 3
    expect(npcFixes).toContain("-- Sessions aggregated: 2");
  });
});
