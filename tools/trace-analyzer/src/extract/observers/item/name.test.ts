import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeName } from "./name";

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

describe("observeName (item)", () => {
  it("should observe an item's id and name directly from a GetLootSlotLink item hyperlink", () => {
    const session = makeSession({
      GetLootSlotLink: {
        "1": [{ t: 3, tp: 3, v: "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }],
      },
    });

    expect(observeName(session)).toEqual([
      {
        entityId: 750,
        value: "Tough Wolf Meat",
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should observe multiple slots across multiple loot events", () => {
    const session = makeSession({
      GetLootSlotLink: {
        "1": [{ t: 0, tp: 0, v: "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }],
        "2": [{ t: 0, tp: 0, v: "|Hitem:2604::::::::1::::::::::|h[Kobold Mining Bag]|h[|r" }],
      },
    });

    expect(observeName(session)).toEqual([
      {
        entityId: 750,
        value: "Tough Wolf Meat",
        confidence: "high",
        provenance: { session: "test-session", t: 0 },
      },
      {
        entityId: 2604,
        value: "Kobold Mining Bag",
        confidence: "high",
        provenance: { session: "test-session", t: 0 },
      },
    ]);
  });

  it("should skip slot entries that aren't a parseable item link", () => {
    const session = makeSession({
      GetLootSlotLink: {
        "1": [{ t: 0, tp: 0, v: null }],
      },
    });

    expect(observeName(session)).toEqual([]);
  });

  it("should return an empty array when no GetLootSlotLink stream exists", () => {
    expect(observeName(makeSession({}))).toEqual([]);
  });
});
