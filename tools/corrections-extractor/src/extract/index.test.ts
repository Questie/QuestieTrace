import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../core/types";
import { createAccumulator, extractAll, finalize, foldSessions } from "./index";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../test/fixtures";

/** Creates a session with a Forever interfaceVersion (16001) */
function makeForeverSession(functions: SessionRecord["functions"]): SessionRecord {
  return makeSession({
    ...functions,
    GetBuildInfo: { player: [apiSample(0, 0, 16001)] },
  });
}

/** Creates a session with a TBC interfaceVersion (10xxx) */
function makeTBCSession(functions: SessionRecord["functions"]): SessionRecord {
  return makeSession({
    ...functions,
    GetBuildInfo: { player: [apiSample(0, 0, 10102)] },
  });
}

describe("extractAll (full chain: observe -> aggregate -> emit -> write)", () => {
  it("should export the name of the newest sessions for a renamed npc", () => {
    const sighting = (name: string, serverTime: number) =>
      makeForeverSession({
        GetLocale: { player: [apiSample(0, 0, "enUS")] },
        GetServerTime: [apiSample(0, 0, serverTime)],
        UnitGUID: { npc: [apiSample(3, 3, creatureGuid(277055))] },
        UnitName: { npc: [apiSample(3, 3, { 1: name, n: 1 })] },
      });

    const { npcFixes } = extractAll(
      [
        sighting("Axle Smoothslice", 1000),
        sighting("Axle Smoothslice", 2000),
        sighting("Axle Smoothslice", 3000),
        sighting("Edine Sharphorn", 4000),
        sighting("Edine Sharphorn", 5000),
      ],
      { sourceFileNames: ["test.lua"], now: new Date("2020-01-01T00:00:00.000Z"), maxIds: { npc: 0, quest: 0, item: 0, object: 0 } },
    );

    expect(npcFixes).toContain('[npcKeys.name] = "Edine Sharphorn",');
    expect(npcFixes).not.toContain("Axle Smoothslice");
  });

  it("should export the level range of an npc seen in several sessions", () => {
    const sighting = (level: number) =>
      makeForeverSession({
        UnitGUID: { target: [apiSample(3, 3, creatureGuid(200001))] },
        UnitLevel: { target: [apiSample(3, 3, level)] },
      });

    const { npcFixes } = extractAll([sighting(12), sighting(12), sighting(13), sighting(13)], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    expect(npcFixes).toContain("[npcKeys.minLevel] = 12,");
    expect(npcFixes).toContain("[npcKeys.maxLevel] = 13,");
  });

  it("should produce a paste-ready foreverNpcTraces.lua correction for an npc encountered in the session", () => {
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { npc: [apiSample(3, 3, "Creature-0-5208-0-7-823-000031")] },
      UnitName: { npc: [apiSample(3, 3, { 1: "Deputy Willem", n: 1 })] },
      "C_Map.GetBestMapForUnit": { player: [apiSample(3, 3, 1436)] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [apiSample(3, 3, { x: 0.3001, y: 0.8602 })] },
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
      UnitGUID: { target: [apiSample(3, 3, "Creature-0-5208-0-7-823-000031")] },
      "C_Map.GetBestMapForUnit": { player: [apiSample(3, 3, 1436)] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [apiSample(3, 3, { x: 0.3001, y: 0.8602 })] },
    });

    const { npcFixes } = extractAll([session], { sourceFileNames: ["test.lua"] });

    // No level was observed, so the levels must not appear in the correction at
    // all - no defaulting to 0/nil.
    expect(npcFixes).not.toContain("npcKeys.minLevel");
    expect(npcFixes).not.toContain("npcKeys.maxLevel");
  });
});

describe("extractAll (quest/item/object entities)", () => {
  it("should produce a paste-ready foreverQuestTraces.lua correction for a quest encountered in the session", () => {
    const session = makeForeverSession({
      GetQuestID: [apiSample(3, 3, 96659)],
      QuestLogZone: { "96659": [apiSample(2, 2, "Westfall")] },
      "C_QuestLog.GetQuestObjectives": {
        "96659": [
          apiSample(3, 3, [
            { type: "item", text: "Tough Wolf Meat: 0/8" },
            { type: "event", text: "Light the campfire", finished: true },
          ]),
        ],
      },
      GetLootSlotLink: {
        "1": [apiSample(4, 4, "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r")],
      },
      GetLootSlotInfo: {
        "1": [apiSample(4, 4, { 2: "Tough Wolf Meat", 6: true, 7: 96659, n: 10 })],
      },
      GetTitleText: [apiSample(3, 3, "A Threat Within")],
      GetLocale: { player: [apiSample(3, 3, "enUS")] },
      "C_Map.GetBestMapForUnit": { player: [apiSample(3, 3, 1436)] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [apiSample(3, 3, { x: 0.3001, y: 0.8602 })] },
      UnitGUID: { target: [apiSample(3, 3, "Creature-0-5208-0-7-823-000031")] },
      UnitName: { target: [apiSample(3, 3, { 1: "Deputy Willem", 2: "", n: 2 })] },
    });

    const { questFixes } = extractAll([session], { sourceFileNames: ["test.lua"], maxIds: { npc: 0, quest: 0, item: 0, object: 0 } });

    expect(questFixes).toContain("function ForeverQuestTraces:Load()");

    expect(questFixes).toContain("function ForeverQuestTraces:Load()");
    expect(questFixes).toContain("[96659] = {");
    expect(questFixes).toContain('[questKeys.name] = "A Threat Within",');
    // zoneOrSort not extracted in tests due to CSV loading limitations
    expect(questFixes).toContain("[questKeys.objectives] = {nil,nil,{{750}}},");
    expect(questFixes).toContain('[questKeys.triggerEnd] = {"Light the campfire",{[40]={{30.01,86.02}}}},');  });

  it("should export objectivesText as a line list, keeping blank separator lines", () => {
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      "GetQuestLogQuestText": {
        "25": [
          apiSample(10, 10, {
            1: "Warlord Senta has a problem...",
            2: "Dispatch 12 Befouled Water Elementals at Mystral Lake.\n\nScout the gazebo on Mystral Lake.\n\nReturn to Mastok Wrilehiss.",
            n: 2,
          }),
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
      GetLootSlotLink: { "1": [apiSample(0, 0, "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r")] },
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      "C_Map.GetBestMapForUnit": { player: [apiSample(0, 0, 1436)] },
      "C_Map.GetPlayerMapPosition": { player: [apiSample(0, 0, { x: 0.3001, y: 0.8602 })] },
    });

    const { itemFixes } = extractAll([session], { sourceFileNames: ["test.lua"], maxIds: { npc: 0, quest: 0, item: 0, object: 0 } });

    expect(itemFixes).toContain("function ForeverItemTraces:Load()");
    expect(itemFixes).toContain("[750] = {");
    expect(itemFixes).toContain('[itemKeys.name] = "Tough Wolf Meat",');
  });

  it("should produce a paste-ready foreverObjectTraces.lua correction for a GameObject encountered in the session", () => {
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { target: [apiSample(3, 3, "GameObject-0-5208-0-7-2843-0000399")] },
      UnitName: { target: [apiSample(3, 3, { 1: "Suspicious Chest", n: 1 })] },
      "C_Map.GetBestMapForUnit": { player: [apiSample(3, 3, 1436)] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [apiSample(3, 3, { x: 0.3001, y: 0.8602 })] },
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
      GetQuestID: [apiSample(3, 3, 96659)],
      QuestLogZone: { "96659": [apiSample(2, 2, "Westfall")] },
      "C_QuestLog.GetQuestObjectives": {
        "96659": [
          apiSample(8, 8, [{ type: "event", text: "Light the campfire", finished: false }]),
          apiSample(12, 12, [{ type: "event", text: "Light the campfire", finished: true }]),
        ],
      },
      "C_Map.GetBestMapForUnit": { player: [apiSample(12, 12, 1429)] },
      "C_Map.GetPlayerMapPosition": { player: [apiSample(12, 12, { x: 0.4746, y: 0.6218 })] },
    });
    const sessionInWestfall = makeForeverSession({
      GetQuestID: [apiSample(3, 3, 96659)],
      QuestLogZone: { "96659": [apiSample(2, 2, "Westfall")] },
      "C_QuestLog.GetQuestObjectives": {
        "96659": [
          apiSample(16, 16, [{ type: "event", text: "Light the campfire", finished: false }]),
          apiSample(20, 20, [{ type: "event", text: "Light the campfire", finished: true }]),
        ],
      },
      "C_Map.GetBestMapForUnit": { player: [apiSample(20, 20, 1436)] },
      "C_Map.GetPlayerMapPosition": { player: [apiSample(20, 20, { x: 0.5611, y: 0.6164 })] },
    });

    const { questFixes } = extractAll([sessionInElwynn, sessionInWestfall], { sourceFileNames: ["test.lua"], maxIds: { npc: 0, quest: 0, item: 0, object: 0 } });

    // zoneOrSort not extracted in tests due to CSV loading limitations
    expect(questFixes).toContain('[questKeys.triggerEnd] = {"Light the campfire",{[40]={{56.11,61.64}}}},');
  });
});

describe("extractAll interfaceVersion filtering", () => {
  it("should include sessions without GetBuildInfo (early Forever beta traces)", () => {
    // Sessions without GetBuildInfo are included because they might be from
    // early Forever beta when PlayerIdentity.lua didn't capture interfaceVersion yet.
    // Any Classic IDs they contain will be filtered by filterBelowMax.
    const sessionWithoutBuildInfo = makeSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { npc: [apiSample(3, 3, "Creature-0-5208-0-7-823-000031")] },
      UnitName: { npc: [apiSample(3, 3, { 1: "Deputy Willem", n: 1 })] },
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
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { npc: [apiSample(3, 3, "Creature-0-5208-0-7-823-000031")] },
      UnitName: { npc: [apiSample(3, 3, { 1: "Deputy Willem", n: 1 })] },
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
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { npc: [apiSample(3, 3, "Creature-0-5208-0-7-823-000031")] },
      UnitName: { npc: [apiSample(3, 3, { 1: "Deputy Willem", n: 1 })] },
      GetBuildInfo: { player: [apiSample(0, 0, 11883)] },
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
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { npc: [apiSample(3, 3, "Creature-0-5208-0-7-823-000031")] },
      UnitName: { npc: [apiSample(3, 3, { 1: "Deputy Willem", n: 1 })] },
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
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { npc: [apiSample(3, 3, "Creature-0-5208-0-7-823-000031")] },
      UnitName: { npc: [apiSample(3, 3, { 1: "Deputy Willem", n: 1 })] },
    });
    const tbcSession = makeTBCSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { npc: [apiSample(3, 3, "Creature-0-5208-0-7-999999")] },
      UnitName: { npc: [apiSample(3, 3, { 1: "TBC NPC", n: 1 })] },
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
    const foreverSession1 = makeForeverSession({ GetLocale: { player: [apiSample(0, 0, "enUS")] } });
    const foreverSession2 = makeForeverSession({ GetLocale: { player: [apiSample(0, 0, "enUS")] } });
    const tbcSession = makeTBCSession({ GetLocale: { player: [apiSample(0, 0, "enUS")] } });

    const { npcFixes } = extractAll([foreverSession1, tbcSession, foreverSession2], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
    });

    // Should report 2 sessions, not 3
    expect(npcFixes).toContain("-- Sessions aggregated: 2");
  });
});

describe("extractAll (integration: no-log 'donation' style quest via gossip)", () => {
  it("should export startedBy for a quest that only appears in C_GossipInfo.GetAvailableQuests (no QUEST_DETAIL/ACCEPTED)", () => {
    // This reproduces the case from trace 0e7c86fe11447abb0dd0e22035b434bf.lua:
    // Quest 99196 "A Donation of Wool" offered by NPC 276171 "Oura Stormspinner"
    // via gossip available quests, with no QUEST_DETAIL or QUEST_ACCEPTED event.
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      "C_GossipInfo.GetAvailableQuests": [
        apiSample(2162.777, 2162.779501, [
          {
            questID: 99196,
            title: "A Donation of Wool",
            questLevel: 0,
            isIgnored: false,
            isImportant: false,
            isMeta: false,
            isLegendary: false,
            repeatable: false,
            frequency: 0,
          },
        ]),
      ],
      UnitGUID: {
        questnpc: [apiSample(2162.777, 2162.779501, "Creature-0-5208-0-7-276171-00002DDB54")],
        npc: [apiSample(2162.777, 2162.779501, "Creature-0-5208-0-7-276171-00002DDB54")],
        target: [apiSample(2162.145, 2162.1480881, "Creature-0-5208-0-7-276171-00002DDB54")],
      },
      UnitName: {
        target: [apiSample(2162.145, 2162.1480881, { 1: "Oura Stormspinner", 2: "", n: 2 })],
      },
    });
    session.events.push(traceEvent("GOSSIP_SHOW", 2162.777, 2162.779));

    const { questFixes, npcFixes } = extractAll([session], {
      sourceFileNames: ["test.lua"],
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    // Quest should have startedBy with the NPC (only creatures array, objects/items omitted)
    expect(questFixes).toContain("[99196] = {");
    expect(questFixes).toContain("[questKeys.startedBy_add] = {{276171}},");

    // NPC should have questStarts with the quest
    expect(npcFixes).toContain("[276171] = {");
    expect(npcFixes).toContain("[npcKeys.questStarts_add] = {99196},");
  });

  it("should export finishedBy and questEnds when QUEST_TURNED_IN is observed (no QUEST_COMPLETE)", () => {
    // Quest completed via QUEST_TURNED_IN (no reward selection frame / QUEST_COMPLETE)
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: {
        questnpc: [apiSample(3, 3, "Creature-0-5208-0-7-276171-00002DDB54")],
        npc: [apiSample(3, 3, "Creature-0-5208-0-7-276171-00002DDB54")],
      },
      UnitName: {
        target: [apiSample(3, 3, { 1: "Oura Stormspinner", 2: "", n: 2 })],
      },
    });

    // Add the QUEST_TURNED_IN event directly to the session events
    const sessionWithTurnIn = {
      ...session,
      events: [
        traceEvent("QUEST_TURNED_IN", 3, 3, 99196),
      ],
    };

    const { questFixes, npcFixes } = extractAll([sessionWithTurnIn], {
      sourceFileNames: ["test.lua"],
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    // Quest should have finishedBy with the NPC (only creatures array, objects omitted)
    expect(questFixes).toContain("[99196] = {");
    expect(questFixes).toContain("[questKeys.finishedBy_add] = {{276171}},");

    // NPC should have questEnds with the quest
    expect(npcFixes).toContain("[276171] = {");
    expect(npcFixes).toContain("[npcKeys.questEnds_add] = {99196},");
  });

  it("should export a single spawn per zone for an npc that finishes a quest", () => {
    const guid = "Creature-0-5208-0-7-276171-00002DDB54";
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: {
        questnpc: [apiSample(3, 3, guid), apiSample(10, 10, guid), apiSample(11, 11, guid)],
      },
      UnitName: {
        target: [apiSample(3, 3, { 1: "Oura Stormspinner", 2: "", n: 2 })],
      },
      "C_Map.GetBestMapForUnit": { player: [apiSample(0, 0, 1436)] },
      "C_Map.GetPlayerMapPosition": {
        player: [
          apiSample(3, 3, { x: 0.1, y: 0.1 }),
          apiSample(10, 10, { x: 0.5, y: 0.5 }),
          apiSample(11, 11, { x: 0.5002, y: 0.5 }),
        ],
      },
    });
    session.events.push(traceEvent("QUEST_TURNED_IN", 3, 3, 99196));

    const { npcFixes } = extractAll([session], {
      sourceFileNames: ["test.lua"],
      maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
    });

    expect(npcFixes).toContain("[npcKeys.spawns] = {[40]={{50.01,50}}},");
  });

  function turnInSession(questId: number) {
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: {
        questnpc: [apiSample(3, 3, "Creature-0-5208-0-7-276171-00002DDB54")],
        target: [apiSample(3, 3, "Creature-0-5208-0-7-276171-00002DDB54")],
      },
      UnitName: {
        target: [apiSample(3, 3, { 1: "Oura Stormspinner", 2: "", n: 2 })],
      },
    });
    return { ...session, events: [traceEvent("QUEST_TURNED_IN", 3, 3, questId)] };
  }

  it("should only export quest relations for an npc below MAX_IDS", () => {
    const options = { sourceFileNames: ["test.lua"], maxIds: { npc: 0, quest: 0, item: 0, object: 0 } };
    expect(extractAll([turnInSession(99196)], options).npcFixes).toContain("[npcKeys.name]");

    const { npcFixes } = extractAll([turnInSession(99196)], { ...options, maxIds: { ...options.maxIds, npc: 300000 } });

    expect(npcFixes).toContain("[npcKeys.questEnds_add] = {99196},");
    expect(npcFixes).not.toContain("[npcKeys.name]");
  });

  it("should export questEnds for an object that finishes a quest", () => {
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { npc: [apiSample(3, 3, "GameObject-0-5208-0-7-410369-00002DDB54")] },
    });
    const withTurnIn = { ...session, events: [traceEvent("QUEST_TURNED_IN", 3, 3, 78261)] };

    const { questFixes, objectFixes } = extractAll([withTurnIn], {
      sourceFileNames: ["test.lua"],
      maxIds: { npc: 0, quest: 0, item: 0, object: 500000 },
    });

    expect(questFixes).toContain("[questKeys.finishedBy_add] = {nil,{410369}},");
    expect(objectFixes).toContain("[410369] = {");
    expect(objectFixes).toContain("[objectKeys.questEnds_add] = {78261},");
  });

  it("should not export relations between a quest and an npc that are both below MAX_IDS", () => {
    const { npcFixes, questFixes } = extractAll([turnInSession(99196)], {
      sourceFileNames: ["test.lua"],
      maxIds: { npc: 300000, quest: 100000, item: 0, object: 0 },
    });

    expect(npcFixes).not.toContain("questEnds_add");
    expect(questFixes).not.toContain("[99196]");
  });

  it("should export only relations to new npcs for a quest below MAX_IDS", () => {
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: {
        questnpc: [apiSample(3, 3, "Creature-0-5208-0-7-276171-00002DDB54")],
        npc: [apiSample(3, 3, "Creature-0-5208-0-7-100-00002DDB54")],
      },
    });
    const withTurnIn = { ...session, events: [traceEvent("QUEST_TURNED_IN", 3, 3, 940)] };

    const { questFixes, npcFixes } = extractAll([withTurnIn], {
      sourceFileNames: ["test.lua"],
      maxIds: { npc: 18199, quest: 9665, item: 0, object: 0 },
    });

    expect(questFixes).toContain("[questKeys.finishedBy_add] = {{276171}},");
    expect(npcFixes).toContain("[276171] = {");
    expect(npcFixes).toContain("[npcKeys.questEnds_add] = {940},");
    expect(npcFixes).not.toContain("[100] = {");
  });

  it("should export only the relation for a quest below MAX_IDS started by a new object", () => {
    const session = makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      UnitGUID: { questnpc: [apiSample(3, 3, "GameObject-0-5208-0-7-673378-00002DDB54")] },
    });
    const withDetail = { ...session, events: [traceEvent("QUEST_ACCEPTED", 3, 3, 940)] };

    const { questFixes, objectFixes } = extractAll([withDetail], {
      sourceFileNames: ["test.lua"],
      maxIds: { npc: 0, quest: 9665, item: 0, object: 300142 },
    });

    expect(questFixes).toContain("[940] = {");
    expect(questFixes).toContain("[questKeys.startedBy_add] = {nil,{673378}},");
    expect(questFixes).not.toContain("[questKeys.name]");
    expect(objectFixes).toContain("[objectKeys.questStarts_add] = {940},");
  });
});

describe("foldSessions + finalize (incremental extraction)", () => {
  const options = {
    sourceFileNames: ["a.lua", "b.lua"],
    now: new Date("2020-01-01T00:00:00.000Z"),
    maxIds: { npc: 0, quest: 0, item: 0, object: 0 },
  };

  function westfallSession(functions: SessionRecord["functions"]): SessionRecord {
    return makeForeverSession({
      GetLocale: { player: [apiSample(0, 0, "enUS")] },
      "C_Map.GetBestMapForUnit": { player: [apiSample(3, 3, 1436)] },
      "C_Map.GetPlayerMapPosition": { player: [apiSample(3, 3, { x: 0.3001, y: 0.8602 })] },
      ...functions,
    });
  }

  const fileA = [
    westfallSession({
      UnitGUID: { npc: [apiSample(3, 3, "Creature-0-5208-0-7-823-000031")] },
      UnitName: { npc: [apiSample(3, 3, { 1: "Deputy Willem", n: 1 })] },
    }),
  ];
  const fileB = [
    westfallSession({
      UnitGUID: { target: [apiSample(3, 3, "GameObject-0-5208-0-7-2843-0000399")] },
      UnitName: { target: [apiSample(3, 3, { 1: "Suspicious Chest", n: 1 })] },
      GetLootSlotLink: { "1": [apiSample(3, 3, "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r")] },
    }),
    makeTBCSession({
      UnitGUID: { npc: [apiSample(3, 3, "Creature-0-5208-0-7-999-000031")] },
      UnitName: { npc: [apiSample(3, 3, { 1: "TBC Only", n: 1 })] },
    }),
  ];

  it("should produce the same corrections when folding files one at a time as extracting all sessions at once", () => {
    const acc = createAccumulator();
    foldSessions(acc, fileA);
    foldSessions(acc, fileB);

    expect(finalize(acc, options)).toEqual(extractAll([...fileA, ...fileB], options));
  });

  it("should only count and observe sessions from Forever builds", () => {
    const acc = createAccumulator();
    foldSessions(acc, fileB);

    expect(acc.sessionCount).toBe(1);
    expect(finalize(acc, options).npcFixes).not.toContain("TBC Only");
  });

  it("should leave the accumulator untouched when an observer throws for a later session", () => {
    const broken = makeForeverSession({});
    broken.functions = new Proxy(broken.functions, {
      get(target, key) {
        if (key === "GetLootSlotLink") throw new Error("corrupt stream");
        return Reflect.get(target, key);
      },
    });
    const acc = createAccumulator();

    expect(() => foldSessions(acc, [...fileA, broken])).toThrow("corrupt stream");
    expect(acc).toEqual(createAccumulator());
  });

  it("should count each unnamed session as a separate objective vote", () => {
    const stormhideSession = (guid: string): SessionRecord => ({
      ...westfallSession({
        "C_QuestLog.GetQuestObjectives": { "33": [{ t: 3, tp: 3, v: [{ type: "monster", text: "0/7 Stormhide slain" }] }] },
        UnitGUID: { target: [{ t: 3, tp: 3, v: guid }] },
        UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Stormhide", n: 1 } }] },
      }),
      name: undefined,
    });

    const { questFixes } = extractAll(
      [
        stormhideSession("Creature-0-5208-0-7-100-000031"),
        stormhideSession("Creature-0-5208-0-7-200-000031"),
        stormhideSession("Creature-0-5208-0-7-200-000032"),
      ],
      options,
    );

    expect(questFixes).toContain("[questKeys.objectives] = {{{200}}},");
  });

  it("should not export objectives when fewer objectives were matched than the API reports", () => {
    const session = westfallSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "monster", text: "0/7 Stormhide slain" }, { type: "item", text: "Empty Vial: 0/5" }] }],
      },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-200-000031" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Stormhide", n: 1 } }] },
    });

    const { questFixes } = extractAll([session], options);

    expect(questFixes).not.toContain("[questKeys.objectives]");
  });
});

describe("extractAll (quest-giver correlation)", () => {
  const relations: Array<{ action: string; gossip: string; questField: string; npcField: string }> = [
    { action: "QUEST_ACCEPTED", gossip: "C_GossipInfo.GetAvailableQuests", questField: "startedBy_add", npcField: "questStarts_add" },
    { action: "QUEST_TURNED_IN", gossip: "C_GossipInfo.GetActiveQuests", questField: "finishedBy_add", npcField: "questEnds_add" },
  ];

  it.each(relations)("should export only the preferred giver on both sides of $questField", ({ action, questField, npcField }) => {
    const session = makeSession({
      UnitGUID: {
        questnpc: [apiSample(3, 3.002, creatureGuid(276171))],
        npc: [apiSample(3, 3.002, creatureGuid(276172))],
      },
    });
    session.events.push(traceEvent(action, 3, 3.001, 99196));

    const output = extractAll([session], { sourceFileNames: [], now: new Date(0) });
    expect(output.questFixes).toContain(`[questKeys.${questField}] = {{276171}}`);
    expect(output.npcFixes).toContain(`[npcKeys.${npcField}] = {99196}`);
    expect(output.questFixes).not.toContain("276172");
    expect(output.npcFixes).not.toContain("276172");
  });

  it.each(relations)("should export a replayed gossip list for both NPCs on $questField", ({ gossip, questField, npcField }) => {
    const session = makeSession({
      UnitGUID: {
        npc: [
          apiSample(3, 3.002, creatureGuid(276171)),
          apiSample(3, 3.012, creatureGuid(276172)),
        ],
      },
      [gossip]: [apiSample(3, 3.002, [{ questID: 99196 }])],
    });
    session.events.push(
      traceEvent("GOSSIP_SHOW", 3, 3.001),
      traceEvent("GOSSIP_SHOW", 3, 3.011),
    );

    const output = extractAll([session], { sourceFileNames: [], now: new Date(0) });
    expect(output.questFixes).toContain(`[questKeys.${questField}] = {{276171,276172}}`);
    expect(output.npcFixes).toMatch(new RegExp(`\\[276171\\] = \\{[^}]*\\[npcKeys\\.${npcField}\\] = \\{99196\\}`));
    expect(output.npcFixes).toMatch(new RegExp(`\\[276172\\] = \\{[^}]*\\[npcKeys\\.${npcField}\\] = \\{99196\\}`));
  });
});
