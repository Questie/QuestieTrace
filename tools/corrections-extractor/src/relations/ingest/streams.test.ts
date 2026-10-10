import { describe, expect, it } from "vitest";
import type { FunctionStreamEntry, SessionRecord } from "../../core/types";
import { entriesBetween, leafStream, sampledBefore, valueAtOrBefore, valueBefore } from "./streams";

// Samples for an event are taken right after it, so "before the next event" must exclude a sample
// stamped exactly at that next event's tp, and "at this read" must include its own tp.
const stream = [
  { t: 1, tp: 1, v: "a" },
  { t: 2, tp: 2, v: "b" },
  { t: 2, tp: 2.5 }, // a nil read
  { t: 3, tp: 3, v: "c" },
];

describe("stream lookups", () => {
  it("valueBefore is strict and valueAtOrBefore inclusive", () => {
    expect(valueBefore(stream, 2)).toBe("a");
    expect(valueAtOrBefore(stream, 2)).toBe("b");
    expect(valueBefore(stream, 1)).toBeUndefined();
  });

  it("entriesBetween includes the start and excludes the end", () => {
    expect(entriesBetween(stream, 2, 3).map((entry) => entry.tp)).toEqual([2, 2.5]);
  });

  it("orders by tp, falling back to t only where tp is missing", () => {
    const frame = [
      { t: 5, tp: 5.2, v: "early" },
      { t: 5, tp: 5.4, v: "late" },
    ];
    expect(valueBefore(frame, 5.3)).toBe("early");
    expect(valueBefore([{ t: 5, v: "legacy" } as FunctionStreamEntry], 5.1)).toBe("legacy");
  });

  it("tells a nil read apart from no read", () => {
    expect(valueBefore(stream, 2.9)).toBeUndefined();
    expect(sampledBefore(stream, 2.9)).toBe(true);
    expect(sampledBefore(stream, 0.5)).toBe(false);
  });

  it("walks parameterized streams and refuses non-leaf nodes", () => {
    const session = { functions: { UnitGUID: { npc: stream } } } as unknown as SessionRecord;
    expect(leafStream(session, "UnitGUID", "npc")).toBe(stream);
    expect(leafStream(session, "UnitGUID")).toBeUndefined();
    expect(leafStream(session, "UnitGUID", "target")).toBeUndefined();
  });
});
