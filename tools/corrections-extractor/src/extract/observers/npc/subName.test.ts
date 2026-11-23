import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { isLevelLine, observeLocalizedSubName, observeSubName } from "./subName";

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

const LOCALE_EN = { GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] } };
const nordun = "Creature-0-5208-0-7-268238-000031";
const isaac = "Creature-0-5208-0-7-258930-000032";

function combatEvent(e: "PLAYER_REGEN_DISABLED" | "PLAYER_REGEN_ENABLED", tp: number): SessionRecord["events"][number] {
  return { t: tp, tp, e } as SessionRecord["events"][number];
}

describe("observeSubName (npc) with recorded guids", () => {
  it("should observe the subname for the guid recorded with it", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Weapon Vendor" }] },
      UnitSubNameGUID: { mouseover: [{ t: 3, tp: 3.1, v: "Creature-0-5208-0-7-823-000031" }] },
    });

    expect(observeSubName(session)).toEqual([
      { entityId: 823, value: "Weapon Vendor", confidence: "high", provenance: { session: "test-session", t: 3 }, token: "mouseover" },
    ]);
  });

  it("should not hand a neighbour's subname to the npc that was sampled at the same time", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitGUID: { mouseover: [{ t: 3, tp: 3, v: nordun }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Isaac Chan's Companion" }] },
      UnitSubNameGUID: { mouseover: [{ t: 3, tp: 3.1, v: isaac }] },
    });

    expect(observeSubName(session).map((observation) => observation.entityId)).toEqual([258930]);
  });

  it("should observe every npc that had the same subname", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitSubName: {
        mouseover: [
          { t: 3, tp: 3.1, v: "Innkeeper" },
          { t: 5, tp: 5.1, v: "Innkeeper" },
        ],
      },
      UnitSubNameGUID: {
        mouseover: [
          { t: 3, tp: 3.1, v: "Creature-0-5208-0-7-823-000031" },
          { t: 5, tp: 5.1, v: "Creature-0-5208-0-7-824-000032" },
        ],
      },
    });

    expect(observeSubName(session).map((observation) => observation.entityId)).toEqual([823, 824]);
  });

  it("should skip subnames without an npc guid", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Innkeeper" }, { t: 4, tp: 4.1, v: "Innkeeper" }] },
      UnitSubNameGUID: { mouseover: [{ t: 3, tp: 3.1, v: "GameObject-0-5208-0-7-2843-0000399" }] },
    });

    expect(observeSubName(session)).toEqual([]);
  });

  it("should keep the locale for the lookup export and skip it for the enUS export", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "deDE" }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Waffenhändler" }] },
      UnitSubNameGUID: { mouseover: [{ t: 3, tp: 3.1, v: "Creature-0-5208-0-7-823-000031" }] },
    });

    expect(observeSubName(session)).toEqual([]);
    expect(observeLocalizedSubName(session).map((observation) => observation.value)).toEqual([{ locale: "deDE", value: "Waffenhändler" }]);
  });

  it("should observe a level line as no subname", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "ruRU" }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Уровень 11 (элита)" }] },
      UnitSubNameGUID: { mouseover: [{ t: 3, tp: 3.1, v: "Creature-0-5208-0-7-823-000031" }] },
    });

    expect(observeLocalizedSubName(session).map((observation) => observation.value)).toEqual([{ locale: "ruRU", value: "" }]);
  });
});

describe("observeSubName (npc) with redacted player names", () => {
  it("should not observe a subname that contains a redacted player name", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "<name> Trainer" }] },
      UnitSubNameGUID: { mouseover: [{ t: 3, tp: 3.1, v: "Creature-0-5208-0-7-823-000031" }] },
    });

    expect(observeSubName(session)).toEqual([]);
  });

  it("should not observe such a subname when inferring the npc from the target either", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitGUID: { target: [{ t: 3.5, tp: 3.5, v: nordun }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "<name>'s Escort" }] },
    });

    expect(observeSubName(session)).toEqual([]);
  });

  it("should still observe a level line with a redacted rank as no subname", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Level 11 (<name>)" }] },
      UnitSubNameGUID: { mouseover: [{ t: 3, tp: 3.1, v: "Creature-0-5208-0-7-823-000031" }] },
    });

    expect(observeSubName(session).map((observation) => observation.value)).toEqual([""]);
  });
});

describe("observeSubName (npc) without recorded guids", () => {
  it("should observe the last subname before the target changes to an npc", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitGUID: { target: [{ t: 3, tp: 3.5, v: nordun }] },
      UnitSubName: {
        mouseover: [
          { t: 1, tp: 1, v: "Isaac Chan's Companion" },
          { t: 3, tp: 3.1, v: "Traveling Ranged Weapon Salesman" },
        ],
      },
    });

    expect(observeSubName(session)).toEqual([
      { entityId: 268238, value: "Traveling Ranged Weapon Salesman", confidence: "high", provenance: { session: "test-session", t: 3 }, token: "target" },
    ]);
  });

  it("should not pair a subname that was recorded more than a second before the target change", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitGUID: { target: [{ t: 4.2, tp: 4.2, v: nordun }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Traveling Ranged Weapon Salesman" }] },
    });

    expect(observeSubName(session)).toEqual([]);
  });

  it("should not pair a subname that was recorded after the target change", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitGUID: { target: [{ t: 3, tp: 3, v: nordun }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Isaac Chan's Companion" }] },
    });

    expect(observeSubName(session)).toEqual([]);
  });

  it("should not pair target changes in combat", () => {
    const events = [combatEvent("PLAYER_REGEN_DISABLED", 2), combatEvent("PLAYER_REGEN_ENABLED", 6)];
    const session = makeSession(
      {
        ...LOCALE_EN,
        UnitGUID: { target: [{ t: 3.5, tp: 3.5, v: nordun }, { t: 6.5, tp: 6.5, v: isaac }] },
        UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Traveling Ranged Weapon Salesman" }, { t: 6.2, tp: 6.2, v: "Isaac Chan's Companion" }] },
      },
      events,
    );

    expect(observeSubName(session).map((observation) => observation.entityId)).toEqual([258930]);
  });

  it("should not pair targets that are not npcs or are cleared", () => {
    const session = makeSession({
      ...LOCALE_EN,
      UnitGUID: { target: [{ t: 3.5, tp: 3.5, v: "GameObject-0-5208-0-7-2843-0000399" }, { t: 3.6, tp: 3.6 }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Innkeeper" }] },
    });

    expect(observeSubName(session)).toEqual([]);
  });

  it("should observe a level line as no subname", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "deDE" }] },
      UnitGUID: { target: [{ t: 3.5, tp: 3.5, v: nordun }] },
      UnitSubName: { mouseover: [{ t: 3, tp: 3.1, v: "Stufe ?? (Boss)" }] },
    });

    expect(observeLocalizedSubName(session).map((observation) => observation.value)).toEqual([{ locale: "deDE", value: "" }]);
  });

  it("should skip sessions without a subname stream", () => {
    const session = makeSession({ ...LOCALE_EN, UnitGUID: { target: [{ t: 3, tp: 3, v: nordun }] } });

    expect(observeSubName(session)).toEqual([]);
  });
});

describe("isLevelLine", () => {
  it.each([
    "Level 28",
    "Level 10 (Elite)",
    "Level ??",
    "Level ?? (Boss)",
    "Stufe 11 (Elite)",
    "Niveau ?? (Élite)",
    "Nivel ?? (Jefe)",
    "Nível 11 (Elite)",
    "Уровень 18",
    "레벨 11 (정예)",
    "等级 10（精英）",
    "等級 11（精英）",
    "<name> 10",
    "Level 11 (<name>)",
  ])("should treat %s as a level line", (subName) => {
    expect(isLevelLine(subName)).toBe(true);
  });

  it.each(["Weapon Vendor", "Waffenhändler", "SI:7 Operative", "Leader of SI:7", "Agent du SI:7", "제2대 칸", "Level Trainer", ""])(
    "should keep %s as a subname",
    (subName) => {
      expect(isLevelLine(subName)).toBe(false);
    },
  );
});
