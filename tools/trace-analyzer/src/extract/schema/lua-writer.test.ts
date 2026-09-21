import { describe, expect, it } from "vitest";
import { writeQuestieLua } from "./lua-writer";
import type { QuestieEntitySchema } from "./questie-keys";

const testSchema: QuestieEntitySchema = {
  name: { index: 1, type: "string", default: "", comment: "" },
  level: { index: 2, type: "int", default: 0, comment: "" },
  spawns: { index: 3, type: "table", default: null, comment: "" },
};

const header = { sourceFileName: "test.lua", sessionCount: 1, generatedAt: new Date("2020-01-01T00:00:00.000Z") };

describe("writeQuestieLua", () => {
  it("should fill every schema field, using extracted values where set and defaults elsewhere", () => {
    const records = new Map([[42, { name: "Deputy Willem", level: 18 }]]);
    const lua = writeQuestieLua("npcData", testSchema, records, header);
    expect(lua).toContain('[42] = {"Deputy Willem",18,nil},');
  });

  it("should fall back to the schema default for every field of an entity with no extracted data", () => {
    const records = new Map([[1, {}]]);
    const lua = writeQuestieLua("npcData", testSchema, records, header);
    expect(lua).toContain('[1] = {"",0,nil},');
  });

  it("should escape double quotes and backslashes in string values", () => {
    const records = new Map([[1, { name: 'He said "hi"\\bye' }]]);
    const lua = writeQuestieLua("npcData", testSchema, records, header);
    expect(lua).toContain('[1] = {"He said \\"hi\\"\\\\bye",0,nil},');
  });

  it("should sort rows by entity id", () => {
    const records = new Map([
      [30, { name: "c" }],
      [10, { name: "a" }],
      [20, { name: "b" }],
    ]);
    const lua = writeQuestieLua("npcData", testSchema, records, header);
    const order = [...lua.matchAll(/\[(\d+)\]/g)].map((m) => Number(m[1]));
    expect(order).toEqual([10, 20, 30]);
  });

  it("should include a header comment with source file, session count, and generation timestamp", () => {
    const lua = writeQuestieLua("npcData", testSchema, new Map(), header);
    expect(lua).toContain("-- Source trace file: test.lua");
    expect(lua).toContain("-- Sessions aggregated: 1");
    expect(lua).toContain("-- Generated at: 2020-01-01T00:00:00.000Z");
    expect(lua).toContain("local npcData = {");
  });
});
