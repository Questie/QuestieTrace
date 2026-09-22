import { describe, expect, it } from "vitest";
import { writeQuestieCorrectionsLua } from "./corrections-writer";

const header = { sourceFileNames: ["test.lua"], sessionCount: 1, generatedAt: new Date("2020-01-01T00:00:00.000Z") };

describe("writeQuestieCorrectionsLua", () => {
  it("should emit a Load() function returning corrections keyed by npcKeys field lookups", () => {
    const records = new Map([[823, { name: "Deputy Willem" }]]);

    const lua = writeQuestieCorrectionsLua("ForeverTraceNpcFixes", "npcKeys", records, header);

    expect(lua).toContain('---@class ForeverTraceNpcFixes');
    expect(lua).toContain('local ForeverTraceNpcFixes = QuestieLoader:CreateModule("ForeverTraceNpcFixes")');
    expect(lua).toContain('local QuestieDB = QuestieLoader:ImportModule("QuestieDB")');
    expect(lua).toContain("function ForeverTraceNpcFixes:Load()");
    expect(lua).toContain("    local npcKeys = QuestieDB.npcKeys");
    expect(lua).toContain('        [823] = {');
    expect(lua).toContain('            [npcKeys.name] = "Deputy Willem",');
    expect(lua).toContain("    }\nend");
  });

  it("should only emit fields actually present on a record, not every schema field", () => {
    const records = new Map([[823, { name: "Deputy Willem", minLevel: 10 }]]);

    const lua = writeQuestieCorrectionsLua("ForeverTraceNpcFixes", "npcKeys", records, header);

    expect(lua).toContain("[npcKeys.name]");
    expect(lua).toContain("[npcKeys.minLevel] = 10,");
    expect(lua).not.toContain("maxLevel");
  });

  it("should skip entities with no corrected fields entirely", () => {
    const records = new Map([
      [1, {}],
      [2, { name: "Has Data" }],
    ]);

    const lua = writeQuestieCorrectionsLua("ForeverTraceNpcFixes", "npcKeys", records, header);

    expect(lua).not.toContain("[1] = {");
    expect(lua).toContain("[2] = {");
  });

  it("should include the header comment with file/session counts and timestamp", () => {
    const lua = writeQuestieCorrectionsLua("ForeverTraceNpcFixes", "npcKeys", new Map(), {
      sourceFileNames: ["a.lua", "b.lua"],
      sessionCount: 5,
      generatedAt: new Date("2020-01-01T00:00:00.000Z"),
    });

    expect(lua).toContain("-- Source trace files: 2");
    expect(lua).toContain("-- Sessions aggregated: 5");
    expect(lua).toContain("-- Generated at: 2020-01-01T00:00:00.000Z");
  });

  it("should escape strings and format nil/number/boolean values correctly", () => {
    const records = new Map([[1, { name: 'Say "Hi"', minLevel: 12, flag: true, missing: null }]]);

    const lua = writeQuestieCorrectionsLua("ForeverTraceNpcFixes", "npcKeys", records, header);

    expect(lua).toContain('[npcKeys.name] = "Say \\"Hi\\"",');
    expect(lua).toContain("[npcKeys.minLevel] = 12,");
    expect(lua).toContain("[npcKeys.flag] = true,");
    expect(lua).toContain("[npcKeys.missing] = nil,");
  });

  it("should return an empty table body when there are no records", () => {
    const lua = writeQuestieCorrectionsLua("ForeverTraceNpcFixes", "npcKeys", new Map(), header);

    expect(lua).toContain("    return {\n    }\nend");
  });
});
