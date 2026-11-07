import { describe, expect, it } from "vitest";
import { writeQuestXpLua } from "./quest-xp-writer";

const header = { sourceFileNames: ["a.json"], sessionCount: 3, generatedAt: new Date("2026-01-01T00:00:00.000Z") };

describe("writeQuestXpLua", () => {
  it("should add one sorted QuestXP.db row per quest", () => {
    const lua = writeQuestXpLua(new Map([[312, { level: 10, xp: 900 }], [99, { level: 5, xp: 110 }]]), header);

    expect(lua).toContain('local QuestXP = QuestieLoader:ImportModule("QuestXP")');
    expect(lua).toContain("-- Sessions aggregated: 3");
    expect(lua.indexOf("QuestXP.db[99] = {5, 110}")).toBeGreaterThan(-1);
    expect(lua.indexOf("QuestXP.db[99]")).toBeLessThan(lua.indexOf("QuestXP.db[312] = {10, 900}"));
  });
});
