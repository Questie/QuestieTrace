import { describe, expect, it } from "vitest";
import type { ObjectiveMatch } from "../../observers/quest/objectives";
import { CREATURE_OBJECTIVE_INDEX, ITEM_OBJECTIVE_INDEX, OBJECT_OBJECTIVE_INDEX, mergeQuestObjectives } from "./objectives";

describe("mergeQuestObjectives", () => {
  function obs(value: ObjectiveMatch[], t = 1, session = "s") {
    return { entityId: 33, value, confidence: "low" as const, provenance: { session, t } };
  }
  function item(id: number, index: number): ObjectiveMatch {
    return { kind: "item", name: `Item ${id}`, text: `Item ${id}: 0/1`, id, index };
  }

  it("should emit a positional table with only the buckets that have data (no leading nils)", () => {
    const matches: ObjectiveMatch[] = [
      { kind: "object", name: "Suspicious Cache", text: "0/1 Suspicious Cache", id: 2843, index: 0 },
      { kind: "item", name: "Tough Wolf Meat", text: "Tough Wolf Meat: 0/8", id: 750, index: 1 },
    ];

    expect(mergeQuestObjectives([obs(matches)])).toEqual({
      [OBJECT_OBJECTIVE_INDEX]: [2843],
      [ITEM_OBJECTIVE_INDEX]: [750],
    });
  });

  it("should dedupe ids for the same objective seen across multiple observations", () => {
    const matches: ObjectiveMatch[] = [
      { kind: "monster", name: "Kobold Vermin", text: "0/10 Kobold Vermin slain", id: 299, index: 0 },
    ];

    expect(mergeQuestObjectives([obs(matches, 1), obs(matches, 2)])).toEqual({ [CREATURE_OBJECTIVE_INDEX]: [299] });
  });

  it("should emit one id per objective when observations resolve the same objective to different ids", () => {
    const hecklefang = (id: number): ObjectiveMatch => ({
      kind: "monster", name: "Hecklefang Stalker", text: "0/7 Hecklefang Stalker slain", id, index: 0,
    });
    const stormhide = (id: number): ObjectiveMatch => ({
      kind: "monster", name: "Stormhide", text: "0/7 Stormhide slain", id, index: 1,
    });

    const result = mergeQuestObjectives([
      obs([hecklefang(4128), stormhide(3456)], 1, "a"),
      obs([hecklefang(4128), stormhide(3238)], 2, "b"),
      obs([stormhide(3238)], 3, "c"),
    ]);

    expect(result).toEqual({ [CREATURE_OBJECTIVE_INDEX]: [4128, 3238] });
  });

  it("should prefer the lowest id when conflicting ids for an objective are seen in equally many sessions", () => {
    const result = mergeQuestObjectives([obs([item(20, 0)], 1, "a"), obs([item(10, 0)], 2, "b")]);

    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([10]);
  });

  it("should count repeated observations from one session only once", () => {
    const result = mergeQuestObjectives([
      obs([item(10, 0)], 1, "a"),
      obs([item(10, 0)], 2, "a"),
      obs([item(10, 0)], 3, "a"),
      obs([item(20, 0)], 4, "b"),
      obs([item(20, 0)], 5, "c"),
    ]);

    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([20]);
  });

  it("should return an empty object when there are no observations", () => {
    expect(mergeQuestObjectives([])).toEqual({});
  });

  it("should order ids by their API objective index, not numerically", () => {
    const result = mergeQuestObjectives([obs([item(251291, 0), item(251284, 1), item(251261, 2)])]);

    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([251291, 251284, 251261]);
  });

  it("should keep API order when observations only matched some objectives", () => {
    // Water(0), Earth(1), Fire(2), Air(3): evidence for each totem shows up at different times.
    const result = mergeQuestObjectives([
      obs([item(274046, 2)], 1),
      obs([item(274044, 1), item(274046, 2), item(274047, 3)], 2),
      obs([item(274045, 0), item(274044, 1), item(274046, 2), item(274047, 3)], 3, "other-session"),
    ]);

    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([274045, 274044, 274046, 274047]);
  });

  it("should keep API order for mixed objective types independently", () => {
    const matches: ObjectiveMatch[] = [
      { kind: "monster", name: "Mob B", text: "0/1 Mob B slain", id: 200, index: 0 },
      { kind: "object", name: "Obj A", text: "0/1 Obj A", id: 100, index: 1 },
      { kind: "monster", name: "Mob A", text: "0/1 Mob A slain", id: 100, index: 2 },
      { kind: "item", name: "Item B", text: "Item B: 0/1", id: 300, index: 3 },
      { kind: "object", name: "Obj B", text: "0/1 Obj B", id: 200, index: 4 },
      { kind: "item", name: "Item A", text: "Item A: 0/1", id: 100, index: 5 },
    ];

    const result = mergeQuestObjectives([obs(matches)]);

    expect(result[CREATURE_OBJECTIVE_INDEX]).toEqual([200, 100]);
    expect(result[OBJECT_OBJECTIVE_INDEX]).toEqual([100, 200]);
    expect(result[ITEM_OBJECTIVE_INDEX]).toEqual([300, 100]);
  });
});
