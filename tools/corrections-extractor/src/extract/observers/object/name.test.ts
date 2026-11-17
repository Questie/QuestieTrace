import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeName } from "./name";

function makeSession(functions: SessionRecord["functions"], events: SessionRecord["events"] = []): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events,
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

  it("should name a looted object by the last tooltip before the loot", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      GameObjectName: [
        { t: 2, tp: 2, v: "Mailbox" },
        { t: 4, tp: 4, v: "Stolen Explosives" },
        { t: 9, tp: 9, v: "Mailbox" },
      ],
      GetLootSourceInfo: { 1: [{ t: 5, tp: 5, v: { 1: "GameObject-0-5208-0-7-673206-0000399", 2: 1, n: 2 } }] },
    }, [{ t: 5, tp: 5, e: "LOOT_READY", a: { n: 0 } }]);

    expect(observeName(session)).toEqual([
      {
        entityId: 673206,
        value: "Stolen Explosives",
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
    ]);
  });

  it("should not name a looted object when no tooltip came before the loot", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      GameObjectName: [{ t: 9, tp: 9, v: "Stolen Explosives" }],
      GetLootSourceInfo: { 1: [{ t: 5, tp: 5, v: { 1: "GameObject-0-5208-0-7-673206-0000399", 2: 1, n: 2 } }] },
    }, [{ t: 5, tp: 5, e: "LOOT_READY", a: { n: 0 } }]);

    expect(observeName(session)).toEqual([]);
  });
});
