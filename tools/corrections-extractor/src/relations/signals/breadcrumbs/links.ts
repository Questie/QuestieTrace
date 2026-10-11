// Which (breadcrumb, target) pairs are worth judging at all, plus the per-quest facts the
// observation pass needs from every episode at once.
//
// A breadcrumb B hands the player over to its target T: B's finisher is T's starter. That link
// comes from the catalog (B's finishers meet T's starters), from the traces (B turned in at a
// giver seen offering T), or from the server popping T's dialog right after B's turn-in. Only
// linked pairs are judged, which keeps the pair count small and unrelated quests out.
//
// A quest offered by one of T's own givers is never T's breadcrumb: a breadcrumb sends the player
// somewhere else (none of Questie's ~215 known breadcrumbs shares a starter with its target), and
// such pairs are mostly siblings in a hub giver's local chain.

import type { GiverRef, QuestCatalog, QuestEpisode } from "../../core/types";

export interface BreadcrumbParams {
  /** A detail this soon after a turn-in at the same giver, with no list in between, is the server's follow-up. */
  autoPopSeconds: number;
  /** A breadcrumb leaving the log this close to the target's accept was dropped by the server. */
  serverDropSeconds: number;
  /** Absence evidence needs the character this many levels above the quest's required level. */
  absenceLevelMargin: number;
}

export const DEFAULT_PARAMS: BreadcrumbParams = {
  autoPopSeconds: 5,
  serverDropSeconds: 2,
  absenceLevelMargin: 1,
};

export interface AutoPop {
  breadcrumb: number;
  target: number;
  turnInT: number;
  detailT: number;
  giver?: GiverRef;
}

export interface Links {
  /** Target -> linked candidate breadcrumbs. */
  byTarget: Map<number, Set<number>>;
  /** Breadcrumb -> linked candidate targets. */
  byBreadcrumb: Map<number, Set<number>>;
  /** Giver ("npc:123") -> quests it starts, by catalog or by observed offers. Gates absence evidence. */
  startsAt: Map<string, Set<number>>;
  /** Quests some character had in its completed set. Without that, "not completed" means nothing. */
  everCompleted: Set<number>;
  /** Repeatable quests: never breadcrumbs, and the client does not flag them completed. */
  repeatable: Set<number>;
}

export function pairKey(breadcrumb: number, target: number): string {
  return `${breadcrumb}>${target}`;
}

export function giverKey(giver: GiverRef): string {
  return `${giver.kind}:${giver.id}`;
}

function addTo<K, V>(map: Map<K, Set<V>>, key: K, value: V): void {
  const set = map.get(key);
  if (set) set.add(value);
  else map.set(key, new Set([value]));
}

/**
 * Turn-ins followed by the next quest's dialog at the same giver without a gossip/greeting list in
 * between: the server's own "next quest" pointer, which both chains and breadcrumbs use.
 */
export function findAutoPops(episode: QuestEpisode, params: BreadcrumbParams): AutoPop[] {
  const result: AutoPop[] = [];
  const listTimes = episode.offers.filter((offer) => offer.source !== "detail").map((offer) => offer.t);
  const events = episode.questEvents;
  for (let i = 0; i < events.length; i++) {
    const turnIn = events[i];
    if (turnIn.kind !== "turnedIn") continue;
    for (let j = i + 1; j < events.length && events[j].t - turnIn.t <= params.autoPopSeconds; j++) {
      const next = events[j];
      if (next.kind === "turnedIn") break;
      if (next.kind !== "detail" || next.questId === turnIn.questId) continue;
      if (turnIn.giver && next.giver && giverKey(turnIn.giver) !== giverKey(next.giver)) break;
      if (listTimes.some((t) => t > turnIn.t && t < next.t)) break;
      result.push({ breadcrumb: turnIn.questId, target: next.questId, turnInT: turnIn.t, detailT: next.t, giver: turnIn.giver ?? next.giver });
      break;
    }
  }
  return result;
}

/**
 * Collects links and global facts in one pass over every episode (observation needs all of them up
 * front), so episodes can be streamed rather than held in memory.
 */
export class LinkCollector {
  private readonly startsAt = new Map<string, Set<number>>();
  private readonly finishedAt = new Map<number, Set<string>>();
  private readonly everCompleted = new Set<number>();
  private readonly repeatable = new Set<number>();
  private readonly autoPopPairs = new Set<string>();

  constructor(
    catalog: QuestCatalog,
    private readonly params: BreadcrumbParams,
  ) {
    for (const quest of Object.values(catalog.quests)) {
      for (const id of quest.starters.npcs) addTo(this.startsAt, `npc:${id}`, quest.id);
      for (const id of quest.starters.objects) addTo(this.startsAt, `object:${id}`, quest.id);
      for (const id of quest.finishers.npcs) addTo(this.finishedAt, quest.id, `npc:${id}`);
      for (const id of quest.finishers.objects) addTo(this.finishedAt, quest.id, `object:${id}`);
      if ((quest.specialFlags ?? 0) & 1) this.repeatable.add(quest.id);
    }
  }

  add(episode: QuestEpisode): void {
    for (const id of episode.completed?.initial ?? []) this.everCompleted.add(id);
    for (const change of episode.completed?.changes ?? []) for (const id of change.add ?? []) this.everCompleted.add(id);
    for (const offer of episode.offers) {
      for (const quest of offer.available) {
        if (quest.repeatable || (quest.frequency ?? 1) > 1) this.repeatable.add(quest.id);
        if (offer.giver && offer.giver.kind !== "item") addTo(this.startsAt, giverKey(offer.giver), quest.id);
      }
    }
    for (const event of episode.questEvents) {
      if (event.kind === "turnedIn" && event.giver && event.giver.kind !== "item") addTo(this.finishedAt, event.questId, giverKey(event.giver));
    }
    for (const pop of findAutoPops(episode, this.params)) this.autoPopPairs.add(pairKey(pop.breadcrumb, pop.target));
  }

  finish(): Links {
    const startedBy = new Map<number, Set<string>>();
    for (const [giver, quests] of this.startsAt) for (const quest of quests) addTo(startedBy, quest, giver);
    const shareStarter = (a: number, b: number) => [...(startedBy.get(a) ?? [])].some((giver) => startedBy.get(b)?.has(giver));

    const byTarget = new Map<number, Set<number>>();
    const byBreadcrumb = new Map<number, Set<number>>();
    const link = (breadcrumb: number, target: number) => {
      if (breadcrumb === target || this.repeatable.has(breadcrumb) || this.repeatable.has(target)) return;
      if (shareStarter(breadcrumb, target)) return;
      addTo(byTarget, target, breadcrumb);
      addTo(byBreadcrumb, breadcrumb, target);
    };
    for (const [breadcrumb, givers] of this.finishedAt) {
      for (const giver of givers) for (const target of this.startsAt.get(giver) ?? []) link(breadcrumb, target);
    }
    for (const key of this.autoPopPairs) {
      const [breadcrumb, target] = key.split(">").map(Number);
      link(breadcrumb, target);
    }
    const { startsAt, everCompleted, repeatable } = this;
    return { byTarget, byBreadcrumb, startsAt, everCompleted, repeatable };
  }
}
