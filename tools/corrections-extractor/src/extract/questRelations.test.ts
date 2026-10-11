import { describe, expect, it } from "vitest";
import { invertQuestRelations } from "./questRelations";

describe("invertQuestRelations", () => {
  it("should invert startedBy and finishedBy onto npcs and objects", () => {
    const quests = new Map<number, Record<string, unknown>>([
      [100, { startedBy: { creatures: [3], objects: [], items: [45] }, finishedBy: { creatures: [], objects: [7] } }],
    ]);
    const { npcs, objects } = invertQuestRelations(quests);
    expect(npcs).toEqual(new Map([[3, { questStarts: [100] }]]));
    expect(objects).toEqual(new Map([[7, { questEnds: [100] }]]));
  });

  it("should put questStarts before questEnds", () => {
    const quests = new Map<number, Record<string, unknown>>([
      [100, { finishedBy: { creatures: [], objects: [7] } }],
      [200, { startedBy: { creatures: [], objects: [7], items: [] } }],
    ]);
    expect(Object.keys(invertQuestRelations(quests).objects.get(7)!)).toEqual(["questStarts", "questEnds"]);
  });

  it("should list quest ids in ascending order", () => {
    const quests = new Map<number, Record<string, unknown>>([
      [200, { startedBy: { creatures: [], objects: [7], items: [] } }],
      [100, { startedBy: { creatures: [], objects: [7], items: [] } }],
    ]);
    expect(invertQuestRelations(quests).objects.get(7)).toEqual({ questStarts: [100, 200] });
  });
});
