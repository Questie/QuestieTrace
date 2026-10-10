// Sightings: moments a character demonstrably could take quest X. The server only offers a
// quest (gossip/greeting list, accept dialog) or lets it be accepted when every prerequisite is
// completed, so the completed set at each sighting must contain all of X's prerequisites.
//
// One CharacterView per (quest, character) condenses that character's sightings of X:
// the quests completed at every sighting (prerequisite candidates), and the timing facts the
// inference step uses to tell a real prerequisite from a quest everyone happened to finish first.

import { isForeverInterface } from "../../core/eligibility";
import { EpisodeTimeline } from "../../core/timeline";
import type { GiverRef, PlayerContext, QuestCatalog, QuestEpisode, SessionTime } from "../../core/types";

/**
 * - offer: listed as available by a giver (any OfferSnapshot, including standalone detail dialogs).
 * - detail / accepted: the accept dialog was shown, or the quest was accepted.
 * - log: first seen in the quest log without an observed accept (accepted before the capture).
 */
export type SightingKind = "offer" | "detail" | "accepted" | "log";

interface Sighting {
  /** Index into the character's episodes, which are ordered by session. */
  episode: number;
  t: SessionTime;
  kind: SightingKind;
  giver?: GiverRef;
}

export interface CharacterView {
  questId: number;
  characterKey: string;
  /** Race and class, to tell "lacked P" from "could never take P" (P is another faction's copy). */
  player: PlayerContext;
  /** Episode and time of the character's first sighting of the quest. */
  episodeKey: string;
  t: SessionTime;
  kind: SightingKind;
  giver?: GiverRef;
  level?: number;
  /** Seconds since the character last levelled up in the session of the first sighting, if it did. */
  levelUpGap?: number;
  sightings: number;
  /** Catalog quests completed at every sighting, ascending: the prerequisite candidates. */
  survivors: number[];
  /** Survivor -> seconds from its completion to the first sighting, for survivors completed earlier in that session. */
  fresh: Map<number, number>;
  /**
   * Set when the character interacted with the quest's giver (any list, dialog or turn-in) before
   * first seeing the quest there. `pending` are the survivors that were not completed yet at that
   * arrival: they became completed while the character was already at the giver.
   */
  arrival?: { level?: number; pending: number[] };
  /**
   * The last complete offer list from the quest's giver before the first sighting. It did not
   * offer the quest (that would have been the first sighting), so whatever survivors were already
   * completed then were not enough on their own; `missing` are the survivors still to come.
   */
  lastList?: { level?: number; missing: number[] };
}

export interface ObserveOptions {
  /** Seconds after a sighting that a completion still counts, to absorb event ordering jitter. */
  completionToleranceSeconds?: number;
}

export interface ObserveStats {
  episodes: number;
  /** Forever episodes with a completed-quest history, grouped into characters. */
  usableEpisodes: number;
  characters: number;
  skippedNotForever: number;
  skippedNoCompleted: number;
  /** Episodes whose completed history is empty although the character is past level 2 (an unloaded history). */
  skippedEmptyHistory: number;
  views: number;
}

/** Quests that are never prerequisite candidates: unknown to the catalog, or repeatable (completion is reset). */
export function candidateFilter(catalog: QuestCatalog): (questId: number) => boolean {
  return (questId) => {
    const quest = catalog.quests[questId];
    return quest !== undefined && ((quest.specialFlags ?? 0) & 1) === 0;
  };
}

export function isUsableEpisode(episode: QuestEpisode): "ok" | "notForever" | "noCompleted" | "emptyHistory" {
  if (!isForeverInterface(episode.interfaceVersion)) return "notForever";
  if (!episode.completed) return "noCompleted";
  const startLevel = episode.levels[0]?.v ?? 0;
  if (episode.completed.initial.length === 0 && startLevel > 2) return "emptyHistory";
  return "ok";
}

/** Usable episodes grouped by character, each character's sessions oldest first (sessionOrder). */
export function groupByCharacter(episodes: Iterable<QuestEpisode>, stats: ObserveStats): Map<string, QuestEpisode[]> {
  const characters = new Map<string, QuestEpisode[]>();
  for (const episode of episodes) {
    stats.episodes++;
    const usable = isUsableEpisode(episode);
    if (usable === "notForever") stats.skippedNotForever++;
    else if (usable === "noCompleted") stats.skippedNoCompleted++;
    else if (usable === "emptyHistory") stats.skippedEmptyHistory++;
    if (usable !== "ok") continue;
    stats.usableEpisodes++;
    const list = characters.get(episode.characterKey);
    if (list) list.push(episode);
    else characters.set(episode.characterKey, [episode]);
  }
  for (const list of characters.values()) {
    list.sort((a, b) => a.sessionOrder - b.sessionOrder);
  }
  stats.characters = characters.size;
  return characters;
}

export function emptyObserveStats(): ObserveStats {
  return { episodes: 0, usableEpisodes: 0, characters: 0, skippedNotForever: 0, skippedNoCompleted: 0, skippedEmptyHistory: 0, views: 0 };
}

function giverKey(giver: GiverRef): string {
  return `${giver.kind}:${giver.id}`;
}

function isEarlier(a: { episode: number; t: number }, b: { episode: number; t: number }): boolean {
  return a.episode < b.episode || (a.episode === b.episode && a.t < b.t);
}

function sightingsOf(episode: QuestEpisode, index: number, add: (questId: number, sighting: Sighting) => void): void {
  for (const offer of episode.offers) {
    for (const quest of offer.available) add(quest.id, { episode: index, t: offer.t, kind: "offer", giver: offer.giver });
  }
  for (const event of episode.questEvents) {
    if (event.kind === "detail" || event.kind === "accepted") {
      add(event.questId, { episode: index, t: event.t, kind: event.kind, giver: event.giver });
    }
  }
  const seenInLog = new Set<number>();
  for (const entry of episode.questLog) {
    for (const questId of entry.v) {
      if (seenInLog.has(questId)) continue;
      seenInLog.add(questId);
      add(questId, { episode: index, t: entry.t, kind: "log" });
    }
  }
}

interface Moment {
  episode: number;
  t: SessionTime;
}

/**
 * Per giver ("npc:123"): the earliest interaction of any kind across the character's sessions, and
 * every complete offer list in session order.
 */
function giverVisits(episodes: QuestEpisode[]): { arrivals: Map<string, Moment>; completeLists: Map<string, Moment[]> } {
  const arrivals = new Map<string, Moment>();
  const completeLists = new Map<string, Moment[]>();
  const visit = (giver: GiverRef | undefined, moment: Moment) => {
    if (!giver || giver.kind === "item") return;
    const key = giverKey(giver);
    const known = arrivals.get(key);
    if (!known || isEarlier(moment, known)) arrivals.set(key, moment);
  };
  episodes.forEach((episode, index) => {
    for (const offer of episode.offers) {
      visit(offer.giver, { episode: index, t: offer.t });
      if (!offer.listComplete || !offer.giver || offer.giver.kind === "item") continue;
      const key = giverKey(offer.giver);
      const lists = completeLists.get(key);
      if (lists) lists.push({ episode: index, t: offer.t });
      else completeLists.set(key, [{ episode: index, t: offer.t }]);
    }
    for (const event of episode.questEvents) visit(event.giver, { episode: index, t: event.t });
  });
  return { arrivals, completeLists };
}

/**
 * The givers whose first visit opens X's "window": the giver seen at the first sighting, else the
 * catalog's starters. Items are skipped: carrying an item says nothing about arriving somewhere.
 */
function windowGivers(first: Sighting, catalog: QuestCatalog, questId: number): string[] {
  if (first.giver) return first.giver.kind === "item" ? [] : [giverKey(first.giver)];
  const starters = catalog.quests[questId]?.starters;
  if (!starters) return [];
  return [...starters.npcs.map((id) => `npc:${id}`), ...starters.objects.map((id) => `object:${id}`)];
}

/** Views for every quest one character was seen able to take. `episodes` must be in session order. */
export function observeCharacter(
  episodes: QuestEpisode[],
  catalog: QuestCatalog,
  options: ObserveOptions = {},
  isCandidate: (questId: number) => boolean = candidateFilter(catalog),
): CharacterView[] {
  const tolerance = options.completionToleranceSeconds ?? 0;
  const timelines = episodes.map((episode) => new EpisodeTimeline(episode));
  const sightings = new Map<number, Sighting[]>();
  episodes.forEach((episode, index) =>
    sightingsOf(episode, index, (questId, sighting) => {
      if (catalog.quests[questId] === undefined) return;
      const list = sightings.get(questId);
      if (list) list.push(sighting);
      else sightings.set(questId, [sighting]);
    }),
  );
  const { arrivals, completeLists } = giverVisits(episodes);

  const views: CharacterView[] = [];
  for (const [questId, list] of sightings) {
    list.sort((a, b) => a.episode - b.episode || a.t - b.t);
    const first = list[0];
    const firstTimeline = timelines[first.episode];

    // Completion only grows (repeatables are filtered out), so the first sighting's set is the
    // smallest; later sightings can still remove a candidate when the data is inconsistent.
    const survivors: number[] = [];
    for (const candidate of firstTimeline.completedAt(first.t + tolerance)) {
      if (candidate === questId || !isCandidate(candidate)) continue;
      if (list.every((s) => timelines[s.episode].isCompletedAt(candidate, s.t + tolerance))) survivors.push(candidate);
    }
    survivors.sort((a, b) => a - b);

    const fresh = new Map<number, number>();
    for (const candidate of survivors) {
      const completedAt = firstTimeline.completionTime(candidate);
      if (typeof completedAt === "number") fresh.set(candidate, Math.max(0, first.t - completedAt));
    }

    const givers = windowGivers(first, catalog, questId);
    let arrival: CharacterView["arrival"];
    let earliest: Moment | undefined;
    for (const key of givers) {
      const visit = arrivals.get(key);
      if (visit && (!earliest || isEarlier(visit, earliest))) earliest = visit;
    }
    if (earliest && isEarlier(earliest, first)) {
      const arrivalTimeline = timelines[earliest.episode];
      arrival = {
        level: arrivalTimeline.levelAt(earliest.t),
        pending: survivors.filter((candidate) => !arrivalTimeline.isCompletedAt(candidate, earliest.t)),
      };
    }

    let lastList: CharacterView["lastList"];
    let latest: Moment | undefined;
    for (const key of givers) {
      for (const list of completeLists.get(key) ?? []) {
        if (isEarlier(list, first) && (!latest || isEarlier(latest, list))) latest = list;
      }
    }
    if (latest) {
      const listTimeline = timelines[latest.episode];
      lastList = { level: listTimeline.levelAt(latest.t), missing: survivors.filter((candidate) => !listTimeline.isCompletedAt(candidate, latest.t)) };
    }

    const levelUp = episodes[first.episode].levels.filter((entry, index) => index > 0 && entry.t <= first.t).at(-1);

    views.push({
      questId,
      characterKey: episodes[first.episode].characterKey,
      player: episodes[first.episode].player,
      episodeKey: episodes[first.episode].key,
      t: first.t,
      kind: first.kind,
      ...(first.giver ? { giver: first.giver } : {}),
      level: firstTimeline.levelAt(first.t),
      ...(levelUp ? { levelUpGap: first.t - levelUp.t } : {}),
      sightings: list.length,
      survivors,
      fresh,
      ...(arrival ? { arrival } : {}),
      ...(lastList ? { lastList } : {}),
    });
  }
  return views;
}
