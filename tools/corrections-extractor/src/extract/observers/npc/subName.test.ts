import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeLocalizedSubName, observeSubName } from "./subName";

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

describe("observeSubName (npc)", () => {
  it("should observe the subname sampled for the mouseover npc at the same time", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { mouseover: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Weapon Vendor" }] },
    });

    expect(observeSubName(session)).toEqual([
      { entityId: 823, value: "Weapon Vendor", confidence: "high", provenance: { session: "test-session", t: 3 } },
    ]);
  });

  it("should not hand the previous npc's subname to an npc whose tooltip was not sampled", () => {
    const session = makeSession(
      {
        GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
        UnitGUID: {
          mouseover: [
            { t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" },
            { t: 5, tp: 5, v: "Creature-0-5208-0-7-824-000032" },
          ],
        },
        UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Weapon Vendor" }] },
      },
      [{ t: 5, tp: 5, e: "UPDATE_MOUSEOVER_UNIT" }] as SessionRecord["events"],
    );

    expect(observeSubName(session).map((observation) => observation.entityId)).toEqual([823]);
  });

  it("should pair a subname sampled in the same frame only with the last npc before it", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: {
        mouseover: [
          { t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" },
          { t: 3, tp: 3.2, v: "Creature-0-5208-0-7-824-000032" },
        ],
      },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.3, v: "Weapon Vendor" }] },
    });

    expect(observeSubName(session).map((observation) => observation.entityId)).toEqual([824]);
  });

  it("should skip encounters where no subname was sampled", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      UnitGUID: { mouseover: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
    });

    expect(observeSubName(session)).toEqual([]);
  });

  it("should keep the locale for the lookup export and skip it for the enUS export", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "deDE" }] },
      UnitGUID: { mouseover: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Waffenhändler" }] },
    });

    expect(observeSubName(session)).toEqual([]);
    expect(observeLocalizedSubName(session).map((observation) => observation.value)).toEqual([{ locale: "deDE", value: "Waffenhändler" }]);
  });
});
