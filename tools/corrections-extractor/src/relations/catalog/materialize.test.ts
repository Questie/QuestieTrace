// Runs forever-quests.lua over inline correction layers through QuestieDB's real registry, so
// the layering rules are checked as the catalog actually sees them. Needs a QuestieDB checkout
// (QUESTIEDB_DIR) and Lua 5.1; skipped where QuestieDB is missing, as in CI.

import { existsSync, mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join, resolve } from "path";
import { beforeAll, describe, expect, it } from "vitest";
import { paths } from "../core/paths";
import { buildCatalog, buildGroundTruth, EXPORTED_FIELDS } from "./build";
import type { MaterializedQuests } from "./materialize";
import { materializeForeverQuests } from "./materialize";

const hasQuestieDb = existsSync(resolve(paths.questieDbDir, "generator/flavor.lua"));

const FIXTURE = `
return function(questKeys)
  return {
    base = {
      [10] = { [questKeys.name] = "Classic", [questKeys.startedBy] = {{1}}, [questKeys.preQuestSingle] = {1, 2},
               [questKeys.exclusiveTo] = {7}, [questKeys.nextQuestInChain] = 5, [questKeys.requiredRaces] = 4294967373 },
    },
    layers = {
      { name = "Forever/legacy/classicQuestFixes.lua:Load", rows = {
        [10] = { [questKeys.preQuestSingle] = {3}, [questKeys.exclusiveTo] = {} },
        [65593] = { [questKeys.name] = "Era event", [questKeys.nextQuestInChain] = 65594 },
      } },
      { name = "Forever/generated/foreverBaseQuest.lua:Load", rows = {
        [9700] = { [questKeys.name] = "New", [questKeys.startedBy] = {{100}} },
      } },
      { name = "Forever/traces/foreverQuestTraces.lua:Load", rows = {
        [9700] = { [questKeys.startedBy_add] = {{101}, nil, {5}} },
        [9701] = { [questKeys.name] = "Traced", [questKeys.startedBy_add] = {{200}} },
      } },
      { name = "Forever/foreverQuestFixes.lua:Load", rows = {
        [10] = { [questKeys.preQuestSingle_add] = {4}, [questKeys.preQuestSingle_remove] = {3} },
        [9700] = { [questKeys.preQuestSingle] = {10} },
      } },
      { name = "Forever/legacy/classicQuestFixes.lua:LoadFactionFixes", dynamic = true, rows = function()
        if UnitFactionGroup("player") == "Horde" then return { [10] = { [questKeys.nextQuestInChain] = 6 } } end
        return {}
      end },
    },
  }
end
`;

describe.skipIf(!hasQuestieDb)("materializeForeverQuests (QuestieDB registry)", () => {
  let materialized: MaterializedQuests;
  beforeAll(() => {
    const fixturePath = join(mkdtempSync(join(tmpdir(), "catalog-")), "fixture.lua");
    writeFileSync(fixturePath, FIXTURE);
    materialized = materializeForeverQuests(paths.questieDbDir, EXPORTED_FIELDS, fixturePath);
  });

  it("applies replace, _add/_remove, {} deletion and per-faction Dynamic Corrections in layer order", () => {
    const truth = buildGroundTruth(materialized, "");
    expect(truth.tiers.inheritedClassic).toEqual({
      // preQuestSingle {1,2} -> replaced by {3} -> authored removes 3, adds 4. exclusiveTo deleted by {}.
      // Horde's Dynamic Correction changes nextQuestInChain; Alliance keeps 5.
      "10": { preQuestSingle: [4], nextQuestInChain: [5, 6] },
      // Created by a legacy fix, so inherited whatever its id.
      "65593": { nextQuestInChain: [65594] },
    });
    // 9700 and 9701 are Forever-new; 9701 has no authored relation, so it is not ground truth.
    expect(truth.tiers.authoredForever).toEqual({ "9700": { preQuestSingle: [10] } });
  });

  it("keeps race bits above 2^32 exact", () => {
    expect(buildCatalog(materialized, "").quests["10"].requiredRaces).toBe(4294967373);
  });

  it("adds trace-derived starters to the base starters", () => {
    const quests = buildCatalog(materialized, "").quests;
    expect(quests["9700"].starters).toEqual({ npcs: [100, 101], objects: [], items: [5] });
    expect(quests["9701"]).toMatchObject({ name: "Traced", starters: { npcs: [200] } });
  });
});
