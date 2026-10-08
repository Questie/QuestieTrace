import { describe, expect, it } from "vitest";
import type { FunctionStreamEntry } from "../core/types";
import { nearestByTime } from "./correlate";

function entries(pairs: Array<[number, unknown]>): FunctionStreamEntry[] {
  return pairs.map(([t, v]) => ({ t, tp: t, v }));
}

describe("nearestByTime", () => {
  it("should return the exact match when an entry exists at targetT", () => {
    const stream = entries([
      [0, "a"],
      [10, "b"],
      [20, "c"],
    ]);
    expect(nearestByTime(stream, 10)).toBe("b");
  });

  it("should return the closest entry when targetT falls between two entries", () => {
    const stream = entries([
      [0, "a"],
      [10, "b"],
    ]);
    expect(nearestByTime(stream, 7)).toBe("b");
    expect(nearestByTime(stream, 3)).toBe("a");
  });

  it("should return null when the closest entry is outside the window", () => {
    const stream = entries([
      [0, "a"],
      [100, "b"],
    ]);
    expect(nearestByTime(stream, 50, 5)).toBeNull();
  });

  it("should return an entry right at the edge of the window", () => {
    const stream = entries([[10, "a"]]);
    expect(nearestByTime(stream, 15, 5)).toBe("a");
    expect(nearestByTime(stream, 15.1, 5)).toBeNull();
  });

  it("should return null for an empty or undefined stream", () => {
    expect(nearestByTime([], 10)).toBeNull();
    expect(nearestByTime(undefined, 10)).toBeNull();
  });
});
