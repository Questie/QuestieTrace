import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import type { FactionReactionValue } from "../../observers/npc/friendlyToFaction";
import { ALLIANCE_RACES, HORDE_RACES, mergeQuestFactions, mergeRequiredRaces, summarizeReactions, type ReactionSummary } from "./requiredRaces";

function reactions(faction: "A" | "H", friendly: boolean, sessions: number, firstSessionKey = 1): Observation<FactionReactionValue>[] {
  return Array.from({ length: sessions }, (_, index) => ({
    entityId: 500,
    value: { faction, friendly, sessionTime: null },
    confidence: "high" as const,
    provenance: { session: "s", sessionKey: `k${firstSessionKey + index}`, t: 0 },
  }));
}

function factionNpc(hostileFaction: "A" | "H", hostileSessions = 3): ReactionSummary {
  const friendlyFaction = hostileFaction === "A" ? "H" : "A";
  return summarizeReactions([...reactions(hostileFaction, false, hostileSessions), ...reactions(friendlyFaction, true, 1, 1000)]);
}

describe("summarizeReactions", () => {
  it("should count hostile and friendly sessions per faction", () => {
    const summary = summarizeReactions([...reactions("H", false, 3), ...reactions("H", false, 1, 3), ...reactions("A", true, 2)]);

    expect(summary).toEqual({ A: { hostileSessions: 0, friendlySessions: 2 }, H: { hostileSessions: 3, friendlySessions: 0 } });
  });
});

describe("mergeRequiredRaces", () => {
  it("should exclude a faction when the quest giver is hostile to it in 3 sessions", () => {
    expect(mergeRequiredRaces(new Set(["A"]), [factionNpc("H")])).toBe(ALLIANCE_RACES);
    expect(mergeRequiredRaces(new Set(["H"]), [factionNpc("A")])).toBe(HORDE_RACES);
  });

  it("should not restrict races when only one faction was observed", () => {
    expect(mergeRequiredRaces(new Set(["A"]), [])).toBeUndefined();
  });

  it("should not exclude a faction when the quest giver was hostile to it in fewer than 3 sessions", () => {
    expect(mergeRequiredRaces(new Set(["A"]), [factionNpc("H", 2)])).toBeUndefined();
  });

  it("should not exclude a faction when any of its sessions saw the quest giver friendly", () => {
    const summary = summarizeReactions([...reactions("H", false, 3), ...reactions("H", true, 1, 100), ...reactions("A", true, 1, 200)]);

    expect(mergeRequiredRaces(new Set(["A"]), [summary])).toBeUndefined();
  });

  it("should not exclude a faction when the quest giver is unfriendly to both factions", () => {
    const summary = summarizeReactions([...reactions("H", false, 3), ...reactions("A", false, 1, 100)]);

    expect(mergeRequiredRaces(new Set(["A"]), [summary])).toBeUndefined();
  });

  it("should not exclude a faction when the quest giver was never seen friendly to the other faction", () => {
    expect(mergeRequiredRaces(new Set(["A"]), [summarizeReactions(reactions("H", false, 3))])).toBeUndefined();
  });

  it("should not exclude a faction when the other faction also saw the quest giver hostile", () => {
    const summary = summarizeReactions([...reactions("H", false, 3), ...reactions("A", true, 5, 100), ...reactions("A", false, 1, 200)]);

    expect(mergeRequiredRaces(new Set(["A"]), [summary])).toBeUndefined();
  });

  it("should use Questie's Forever faction masks", () => {
    expect(ALLIANCE_RACES).toBe(4294967373);
    expect(HORDE_RACES).toBe(8589934770);
  });

  it("should not exclude a faction when any of its sessions saw the quest", () => {
    expect(mergeRequiredRaces(new Set(["A", "H"]), [factionNpc("H")])).toBeUndefined();
  });

  it("should not exclude a faction when another quest giver might be friendly to it", () => {
    expect(mergeRequiredRaces(new Set(["A"]), [factionNpc("H"), summarizeReactions([])])).toBeUndefined();
  });

  it("should not restrict races when no session of the remaining faction saw the quest", () => {
    expect(mergeRequiredRaces(new Set([]), [factionNpc("H")])).toBeUndefined();
  });

  it("should not exclude a faction that saw another quest of the same quest giver", () => {
    expect(mergeRequiredRaces(new Set(["A"]), [factionNpc("H")], new Set(["H"]))).toBeUndefined();
  });

  it("should exclude a faction when only the remaining faction saw other quests of the same quest giver", () => {
    expect(mergeRequiredRaces(new Set(["A"]), [factionNpc("H")], new Set(["A"]))).toBe(ALLIANCE_RACES);
  });
});

describe("mergeQuestFactions", () => {
  it("should collect the factions of all observations", () => {
    const observations: Observation<"A" | "H">[] = [
      { entityId: 1, value: "A", confidence: "high", provenance: { session: "s", t: 0 } },
      { entityId: 1, value: "H", confidence: "high", provenance: { session: "s", t: 1 } },
    ];

    expect(mergeQuestFactions(observations)).toEqual(new Set(["A", "H"]));
  });
});
