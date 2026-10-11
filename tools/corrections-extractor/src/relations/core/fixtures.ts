// Test helpers for building small episodes and catalogs by hand. Only test files import this.

import type { CatalogQuest, GiverRef, OfferSnapshot, QuestEpisode, QuestEvent } from "./types";

let nextEpisode = 0;

/** An episode with sensible defaults; override only what the test cares about. */
export function makeEpisode(overrides: Partial<QuestEpisode> = {}): QuestEpisode {
  nextEpisode++;
  return {
    key: `test-${nextEpisode}`,
    contributorId: "contributor",
    characterKey: `character-${nextEpisode}`,
    submissionIds: [`submission-${nextEpisode}`],
    sessionOrder: 0,
    interfaceVersion: 16001,
    locale: "enUS",
    player: { raceId: 1, classId: 1, faction: "Alliance" },
    duration: 3600,
    levels: [{ t: 0, v: 10 }],
    completed: { source: "modern", initial: [], changes: [] },
    questLog: [{ t: 0, v: [] }],
    questEvents: [],
    offers: [],
    ...overrides,
  };
}

export function npc(id: number): GiverRef {
  return { kind: "npc", id };
}

export function event(t: number, kind: QuestEvent["kind"], questId: number, giver?: GiverRef): QuestEvent {
  return giver ? { t, kind, questId, giver } : { t, kind, questId };
}

/** A complete gossip offer list. */
export function gossip(t: number, giver: GiverRef, available: number[], active: number[] = []): OfferSnapshot {
  return { t, source: "gossip", giver, available: available.map((id) => ({ id })), active, listComplete: true };
}

export function catalogQuest(id: number, overrides: Partial<CatalogQuest> = {}): CatalogQuest {
  return {
    id,
    starters: { npcs: [], objects: [], items: [] },
    finishers: { npcs: [], objects: [] },
    ...overrides,
  };
}
