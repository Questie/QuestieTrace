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

describe("observeName (object)", () => {
  it("should observe the object's name at the same time/token as the GUID encounter", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Suspicious Chest", n: 2 } }] },
    });

    expect(observeName(session)).toEqual([
      {
        entityId: 2843,
        value: "Suspicious Chest",
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should skip encounters where no name was observed", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }] },
    });

    expect(observeName(session)).toEqual([]);
  });

  it("should skip encounters with non-enUS locale", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "frFR" }] },
      UnitGUID: { target: [{ t: 3, tp: 3, v: "GameObject-0-5208-0-7-2843-0000399" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Coffre suspect", n: 2 } }] },
    });

    expect(observeName(session)).toEqual([]);
  });
});
