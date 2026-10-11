// Folds every character's view of quest X into per-candidate counts. A candidate P stays
// "alive" while (almost) every character had it completed at every sighting of X; the other
// counts measure how specific P is to X, because quests everyone finishes early survive too.

import { classAllowed, raceAllowed } from "../../core/eligibility";
import type { Evidence, QuestCatalog } from "../../core/types";
import type { CharacterView } from "./observe";
import type { VariantIndex } from "./variants";

export interface AggregateParams {
  /** A completion this many seconds or less before the first sighting counts as "fresh". */
  freshSeconds: number;
  /** ...and this many seconds or less as "immediate" (turned in, next quest offered right away). */
  immediateSeconds: number;
  /** Completions this close to the most recent one share "latest" (variants completed together). */
  latestTieSeconds: number;
  /** Characters allowed to lack P at a sighting before P is dropped (absorbs bad histories). */
  maxContradictions: number;
}

export interface CandidateStats {
  target: number;
  /** Characters that had the target completed at every sighting of the quest. */
  support: number;
  /** Characters that saw the quest at least once without the target completed. */
  contradict: number;
  /** Characters that completed the target earlier in the session of their first sighting, within freshSeconds. */
  fresh: number;
  immediate: number;
  /** Characters for which the target was the most recently completed survivor (within freshSeconds) before the first sighting. */
  latest: number;
  /** Characters that completed the target after arriving at the quest's giver, at a level that allowed the quest. */
  pending: number;
  /** Characters that already had the target completed when they first reached the quest's giver. */
  background: number;
  /** The target is turned in (per catalog or traces) at one of the quest's starters or the giver it was seen at. */
  link: boolean;
  evidence: Evidence[];
}

export interface OrGroup {
  /** Audience-exclusive same-name variants, ascending. */
  members: number[];
  /** Summed over members; support counts characters with any member at every sighting. */
  stats: CandidateStats;
  perMember: Map<number, number>;
}

export interface QuestAssessment {
  questId: number;
  /** Independent characters with at least one sighting. */
  characters: number;
  /** Characters whose first sighting followed an earlier visit to the giver at a sufficient level. */
  windowCharacters: number;
  /**
   * Characters that first saw the quest right after reaching its required level: for them the
   * quest may have unlocked by levelling, so whatever they completed just before proves little.
   */
  levelUnlocked: number;
  /**
   * Per character whose last complete list from the giver (before first seeing the quest) came at
   * a level that allowed it: the survivors not completed yet at that list. The list did not offer
   * the quest, so at least one of them (or something untracked) was still required.
   */
  blockedLists: number[][];
  /** Candidates with at most maxContradictions contradictions. */
  alive: CandidateStats[];
  /** Variant groups that covered every character although no single member did. */
  orGroups: OrGroup[];
}

/** Giver keys ("npc:123") a quest is turned in at. */
export type FinishersOf = (questId: number) => ReadonlySet<string>;

const MAX_EVIDENCE = 5;

function emptyStats(target: number): CandidateStats {
  return { target, support: 0, contradict: 0, fresh: 0, immediate: 0, latest: 0, pending: 0, background: 0, link: false, evidence: [] };
}

function questGivers(views: CharacterView[], catalog: QuestCatalog, questId: number): Set<string> {
  const ids = new Set<string>();
  const starters = catalog.quests[questId]?.starters;
  for (const id of starters?.npcs ?? []) ids.add(`npc:${id}`);
  for (const id of starters?.objects ?? []) ids.add(`object:${id}`);
  for (const view of views) if (view.giver && view.giver.kind !== "item") ids.add(`${view.giver.kind}:${view.giver.id}`);
  return ids;
}

function describeSighting(view: CharacterView): string {
  const where = view.giver ? ` at ${view.giver.kind} ${view.giver.id}` : "";
  return `${view.questId} first seen (${view.kind})${where}, level ${view.level ?? "?"}`;
}

/** Catalog finishers plus the givers traces saw the quest turned in at. */
export function finisherIndex(catalog: QuestCatalog, turnIns: ReadonlyMap<number, ReadonlySet<string>>): FinishersOf {
  const cache = new Map<number, Set<string>>();
  return (questId) => {
    let set = cache.get(questId);
    if (!set) {
      const finishers = catalog.quests[questId]?.finishers;
      set = new Set([...(finishers?.npcs ?? []).map((id) => `npc:${id}`), ...(finishers?.objects ?? []).map((id) => `object:${id}`)]);
      for (const key of turnIns.get(questId) ?? []) set.add(key);
      cache.set(questId, set);
    }
    return set;
  };
}

/** Aggregates the views of one quest (one view per character). */
export function aggregateQuest(
  questId: number,
  views: CharacterView[],
  catalog: QuestCatalog,
  params: AggregateParams,
  finishersOf: FinishersOf,
  variants: VariantIndex,
): QuestAssessment {
  const givers = questGivers(views, catalog, questId);
  let minLevel: number | undefined;
  for (const view of views) if (view.level !== undefined && (minLevel === undefined || view.level < minLevel)) minLevel = view.level;
  const levelGate = catalog.quests[questId]?.requiredLevel ?? minLevel ?? 0;

  const stats = new Map<number, CandidateStats>();
  let windowCharacters = 0;
  let levelUnlocked = 0;
  const blockedLists: number[][] = [];
  for (const view of views) {
    if (view.lastList && (view.lastList.level ?? 0) >= levelGate && view.lastList.missing.length > 0) blockedLists.push(view.lastList.missing);
    if (view.levelUpGap !== undefined && view.levelUpGap <= params.freshSeconds && view.level === levelGate) levelUnlocked++;
    // A window only says something when the level already allowed the quest at arrival:
    // otherwise the quest may simply have unlocked by levelling up.
    const window = view.arrival && (view.arrival.level ?? 0) >= levelGate ? new Set(view.arrival.pending) : undefined;
    if (window) windowCharacters++;

    let latestGap = Infinity;
    for (const gap of view.fresh.values()) latestGap = Math.min(latestGap, gap);

    for (const target of view.survivors) {
      let entry = stats.get(target);
      if (!entry) stats.set(target, (entry = emptyStats(target)));
      entry.support++;
      const gap = view.fresh.get(target);
      if (gap !== undefined && gap <= params.freshSeconds) {
        entry.fresh++;
        if (gap <= params.immediateSeconds) entry.immediate++;
        if (gap <= latestGap + params.latestTieSeconds) entry.latest++;
      }
      if (window) {
        if (window.has(target)) entry.pending++;
        else entry.background++;
      }
      if (gap !== undefined && entry.evidence.length < MAX_EVIDENCE) {
        entry.evidence.push({ ref: view.episodeKey, t: view.t, text: `${target} completed ${gap.toFixed(1)}s before ${describeSighting(view)}` });
      }
    }
  }

  const characters = views.length;
  const isLinked = (target: number) => [...finishersOf(target)].some((key) => givers.has(key));
  const alive: CandidateStats[] = [];
  for (const entry of stats.values()) {
    entry.contradict = characters - entry.support;
    if (entry.contradict > params.maxContradictions) continue;
    entry.link = isLinked(entry.target);
    alive.push(entry);
  }
  alive.sort((a, b) => a.target - b.target);

  return { questId, characters, windowCharacters, levelUnlocked, blockedLists, alive, orGroups: orGroups(views, stats, catalog, params, isLinked, variants) };
}

/** Binary search: survivors are ascending. */
function survived(view: CharacterView, target: number): boolean {
  const list = view.survivors;
  let lo = 0;
  let hi = list.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (list[mid] === target) return true;
    if (list[mid] < target) lo = mid + 1;
    else hi = mid - 1;
  }
  return false;
}

/**
 * Faction-split prerequisites: the Alliance characters had P1, the Horde ones P2. Each member is
 * contradicted overall, but only by characters that could never take it (race/class), the members
 * are pairwise audience-exclusive same-name variants, and together they cover every character.
 */
function orGroups(
  views: CharacterView[],
  stats: Map<number, CandidateStats>,
  catalog: QuestCatalog,
  params: AggregateParams,
  isLinked: (target: number) => boolean,
  variants: VariantIndex,
): OrGroup[] {
  const canTake = (target: number, view: CharacterView) => {
    const quest = catalog.quests[target];
    return raceAllowed(quest?.requiredRaces, view.player.raceId) && classAllowed(quest?.requiredClasses, view.player.classId);
  };
  const partial = new Map<number, boolean>();
  const isPartial = (target: number): boolean => {
    let known = partial.get(target);
    if (known === undefined) {
      const entry = stats.get(target);
      known =
        entry !== undefined &&
        entry.contradict > params.maxContradictions &&
        views.filter((view) => canTake(target, view) && !survived(view, target)).length <= params.maxContradictions;
      partial.set(target, known);
    }
    return known;
  };

  const result = new Map<string, OrGroup>();
  for (const entry of stats.values()) {
    const others = variants.variantsOf(entry.target);
    if (others.length === 0 || !isPartial(entry.target)) continue;
    // Greedy by support keeps one copy per audience (a chain of same-name steps shares an audience).
    const members = [entry.target];
    for (const other of others.filter(isPartial).sort((a, b) => stats.get(b)!.support - stats.get(a)!.support)) {
      if (members.every((member) => variants.exclusiveAudiences(member, other))) members.push(other);
    }
    members.sort((a, b) => a - b);
    const key = members.join(",");
    if (members.length < 2 || result.has(key)) continue;

    const summed = emptyStats(members[0]);
    const perMember = new Map<number, number>();
    summed.support = views.filter((view) => members.some((member) => survived(view, member))).length;
    summed.contradict = views.length - summed.support;
    if (summed.contradict > params.maxContradictions) continue;
    for (const member of members) {
      const own = stats.get(member)!;
      perMember.set(member, own.support);
      for (const field of ["fresh", "immediate", "latest", "pending", "background"] as const) summed[field] += own[field];
      summed.link ||= isLinked(member);
      summed.evidence.push(...own.evidence.slice(0, 2));
    }
    result.set(key, { members, stats: summed, perMember });
  }
  return [...result.values()];
}
