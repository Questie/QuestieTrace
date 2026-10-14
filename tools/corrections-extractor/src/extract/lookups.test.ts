import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../core/types";
import { createLookupAccumulator, finalizeLookups, foldLookupSessions } from "./lookups";

function makeSession(functions: SessionRecord["functions"]): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    events: [],
    functions,
    functionsDelta: {},
  };
}

function itemSession(locale: string, link: string, interfaceVersion = 16001): SessionRecord {
  return makeSession({
    GetBuildInfo: { player: [{ t: 0, tp: 0, v: interfaceVersion }] },
    GetLocale: { player: [{ t: 0, tp: 0, v: locale }] },
    GetLootSlotLink: { 1: [{ t: 1, tp: 1, v: link }] },
  });
}

const ITEM_LINK_DE = "|cffffffff|Hitem:30000::::::::1:::::::|h[Schwert]|h|r";
const ITEM_LINK_FR = "|cffffffff|Hitem:30000::::::::1:::::::|h[Épée]|h|r";

function run(sessions: SessionRecord[]) {
  const acc = createLookupAccumulator();
  foldLookupSessions(acc, sessions);
  return finalizeLookups(acc);
}

describe("finalizeLookups", () => {
  it("should write one item lookup per non-enUS locale", () => {
    const bundle = run([itemSession("deDE", ITEM_LINK_DE), itemSession("frFR", ITEM_LINK_FR)]);

    expect(bundle.lookupItems.deDE).toBe(
      [
        'if GetLocale() ~= "deDE" then',
        "    return",
        "end",
        "",
        "---@type l10n",
        'local l10n = QuestieLoader:ImportModule("l10n")',
        "",
        'l10n.itemLookup["deDE"] = loadstring([[return {',
        '[30000] = "Schwert",',
        "}]])",
        "",
      ].join("\n"),
    );
    expect(bundle.lookupItems.frFR).toContain('[30000] = "Épée",');
  });

  it("should skip enUS, non-Forever sessions and known classic IDs", () => {
    const bundle = run([
      itemSession("enUS", ITEM_LINK_DE),
      itemSession("frFR", ITEM_LINK_FR, 11507),
      itemSession("deDE", "|cffffffff|Hitem:25::::::::1:::::::|h[Kurzschwert]|h|r"),
    ]);

    expect(bundle.lookupItems).toEqual({});
  });

  it("should write quest titles together with their objectives text", () => {
    const bundle = run([
      makeSession({
        GetLocale: { player: [{ t: 0, tp: 0, v: "deDE" }] },
        GetQuestID: [{ t: 3, tp: 3, v: 90001 }],
        GetTitleText: [{ t: 3, tp: 3, v: "Eine Quest" }],
        GetQuestLogQuestText: { 90001: [{ t: 4, tp: 4, v: { 1: "Beschreibung", 2: "Tötet 5.\n\nKehrt zurück.", n: 2 } }] },
      }),
    ]);

    expect(bundle.lookupQuests.deDE).toContain('l10n.questLookup["deDE"] = loadstring([[return {');
    expect(bundle.lookupQuests.deDE).toContain('[90001] = {"Eine Quest", {"Tötet 5.","","Kehrt zurück."}},');
  });

  it("should write npc names with a nil subname", () => {
    const bundle = run([
      makeSession({
        GetLocale: { player: [{ t: 0, tp: 0, v: "deDE" }] },
        UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-30000-000031" }] },
        UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Koboldgezücht", n: 2 } }] },
      }),
    ]);

    expect(bundle.lookupNpcs.deDE).toContain('l10n.npcNameLookup["deDE"] = loadstring([[return {');
    expect(bundle.lookupNpcs.deDE).toContain('[30000] = {"Koboldgezücht",nil},');
  });

  it("should leave the accumulator untouched when an observer throws for a later session", () => {
    const broken = itemSession("frFR", ITEM_LINK_FR);
    broken.functions = new Proxy(broken.functions, {
      get(target, key) {
        if (key === "GetLootSlotLink") throw new Error("corrupt stream");
        return Reflect.get(target, key);
      },
    });
    const acc = createLookupAccumulator();

    expect(() => foldLookupSessions(acc, [itemSession("deDE", ITEM_LINK_DE), broken])).toThrow("corrupt stream");
    expect(acc).toEqual(createLookupAccumulator());
  });
});
