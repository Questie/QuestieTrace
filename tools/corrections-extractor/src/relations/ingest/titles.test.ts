import { describe, expect, it } from "vitest";
import type { OfferSnapshot } from "../core/types";
import { emptyGreetingStats, resolveGreeting, TitleIndex } from "./titles";

const giver = { kind: "npc" as const, id: 240 };

function index(): TitleIndex {
  const titles = new TitleIndex();
  // "Manhunt" names two quests in this locale, but only one at this giver.
  titles.add({ locale: "enUS", faction: "Alliance", title: "Manhunt", questId: 147, giver: "npc:240" });
  titles.add({ locale: "enUS", faction: "Alliance", title: "Manhunt", questId: 9000, giver: "npc:999" });
  titles.add({ locale: "enUS", faction: "Alliance", title: "Westbrook Garrison Needs Help!", questId: 239 });
  titles.add({ locale: "enUS", faction: "Horde", title: "Report to Kadrak", questId: 6541 });
  titles.add({ locale: "enUS", faction: "Alliance", title: "Threat from Below", questId: 99051, giver: "npc:240" });
  titles.add({ locale: "enUS", faction: "Alliance", title: "Threat from Below", questId: 99052, giver: "npc:240" });
  for (const contributor of ["a", "b"]) {
    for (const title of ["Threat from Below", "Report to Kadrak", "Manhunt"]) titles.noteSighting("enUS", title, contributor);
  }
  // Seen by one contributor only, e.g. an older trace's unsanitized title naming the player.
  titles.noteSighting("enUS", "Never Seen", "a");
  return titles;
}

function greeting(available: string[], expected = available.length, active: string[] = []): OfferSnapshot {
  const offer: OfferSnapshot = { t: 1, source: "greeting", giver, available: [], active: [], listComplete: false };
  resolveGreeting(offer, { offer: 0, available, active, expected }, { locale: "enUS", faction: "Alliance" }, index(), emptyGreetingStats());
  return offer;
}

describe("resolveGreeting", () => {
  it("resolves titles at the giver first, then among the same faction's pairings", () => {
    expect(greeting(["Manhunt", "Westbrook Garrison Needs Help!"])).toEqual({
      t: 1,
      source: "greeting",
      giver,
      available: [{ id: 147 }, { id: 239 }],
      active: [],
      listComplete: true,
    });
  });

  it("keeps titles that are unknown, ambiguous or only seen by the other faction unresolved", () => {
    const offer = greeting(["Threat from Below", "Report to Kadrak"]);
    expect(offer.available).toEqual([]);
    expect(offer.unresolvedTitles).toEqual(["Threat from Below", "Report to Kadrak"]);
    expect(offer.listComplete).toBe(false);
  });

  it("withholds unresolved titles only one contributor ever saw", () => {
    const offer = greeting(["Report to Kadrak", "Never Seen"]);
    expect(offer.unresolvedTitles).toEqual(["Report to Kadrak"]);
    expect(offer.listComplete).toBe(false);
  });

  it("is not complete when titles were missing from the trace or two titles map to one quest", () => {
    expect(greeting(["Manhunt"], 2).listComplete).toBe(false);
    expect(greeting(["Manhunt", "Manhunt"]).listComplete).toBe(false);
  });

  it("completes a list that mixes quest ids read directly with titles", () => {
    const offer: OfferSnapshot = { t: 1, source: "greeting", giver, available: [{ id: 147, trivial: false }], active: [123], listComplete: false };
    resolveGreeting(offer, { offer: 0, available: ["Westbrook Garrison Needs Help!"], active: [], expected: 2 }, { locale: "enUS", faction: "Alliance" }, index(), emptyGreetingStats());
    expect(offer).toMatchObject({ available: [{ id: 147, trivial: false }, { id: 239 }], active: [123], listComplete: true });
  });

  it("resolves the active list too", () => {
    expect(greeting([], 0, ["Manhunt", "Never Seen"])).toMatchObject({ active: [147], listComplete: true });
  });
});
