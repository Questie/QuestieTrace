// Resolves legacy QUEST_GREETING titles to quest ids.
//
// Older traces record a greeting list as titles only (GetAvailableTitle per index). Every
// submission also pairs titles with quest ids elsewhere: GetTitleText + GetQuestID in quest
// dialogs, gossip list entries, and quest log rows. A title resolves when those pairings name
// exactly one quest, first among pairings seen at the same giver, then among everything the
// same faction saw in that locale.
// Titles are only held here transiently; episodes keep just the ones left unresolved, and only
// when several contributors saw them: older traces did not sanitize greeting titles, and a title
// that names the player is unique to that player.

import type { GiverRef, OfferSnapshot } from "../core/types";
import { giverKey, type PendingGreeting, type TitleObservation } from "./types";

export interface GreetingStats {
  greetings: number;
  completeGreetings: number;
  titlesResolved: number;
  titlesUnresolved: number;
  /** Unresolved titles left out of episodes because fewer than SHARED_BY contributors saw them. */
  titlesWithheld: number;
  activeResolved: number;
  activeUnresolved: number;
}

export function emptyGreetingStats(): GreetingStats {
  return { greetings: 0, completeGreetings: 0, titlesResolved: 0, titlesUnresolved: 0, titlesWithheld: 0, activeResolved: 0, activeUnresolved: 0 };
}

const SEP = "\u0000";
/** Contributors that must have seen a title before it may appear in an episode. */
const SHARED_BY = 2;

export class TitleIndex {
  private readonly atGiver = new Map<string, Set<number>>();
  private readonly inLocale = new Map<string, Set<number>>();
  private readonly sightings = new Map<string, Set<string>>();

  /** Records that a contributor saw a title anywhere: dialog, gossip list, quest log or greeting. */
  noteSighting(locale: string | undefined, title: string, contributorId: string): void {
    if (!locale) return;
    const key = `${locale}${SEP}${normalizeTitle(title)}`;
    const seen = this.sightings.get(key);
    if (!seen) this.sightings.set(key, new Set([contributorId]));
    else if (seen.size < SHARED_BY) seen.add(contributorId);
  }

  /** Whether enough contributors saw the title that it cannot be text about one player. */
  isShared(locale: string | undefined, title: string): boolean {
    return !!locale && (this.sightings.get(`${locale}${SEP}${normalizeTitle(title)}`)?.size ?? 0) >= SHARED_BY;
  }

  add(observation: TitleObservation): void {
    const title = normalizeTitle(observation.title);
    addTo(this.inLocale, `${observation.locale}${SEP}${observation.faction ?? ""}${SEP}${title}`, observation.questId);
    if (observation.giver) addTo(this.atGiver, `${observation.locale}${SEP}${observation.giver}${SEP}${title}`, observation.questId);
  }

  /** The single quest id this title names (at this giver, else for this locale and faction), or undefined when unknown or ambiguous. */
  resolve(locale: string | undefined, faction: string | undefined, giver: GiverRef | undefined, title: string): number | undefined {
    if (!locale) return undefined;
    const normalized = normalizeTitle(title);
    if (giver) {
      const ids = this.atGiver.get(`${locale}${SEP}${giverKey(giver)}${SEP}${normalized}`);
      if (ids) return ids.size === 1 ? only(ids) : undefined;
    }
    const ids = this.inLocale.get(`${locale}${SEP}${faction ?? ""}${SEP}${normalized}`);
    return ids?.size === 1 ? only(ids) : undefined;
  }
}

/** Adds the quests behind a greeting's titles to its snapshot (which may already hold quest ids read directly). */
export function resolveGreeting(
  offer: OfferSnapshot,
  pending: PendingGreeting,
  context: { locale?: string; faction?: string },
  index: TitleIndex,
  stats: GreetingStats,
): void {
  const ids = offer.available.map((quest) => quest.id);
  const unresolved: string[] = [];
  for (const title of pending.available) {
    const id = index.resolve(context.locale, context.faction, offer.giver, title);
    if (id) ids.push(id);
    else unresolved.push(title);
  }
  const active = new Set(offer.active);
  for (const title of pending.active) {
    const id = index.resolve(context.locale, context.faction, offer.giver, title);
    if (id) active.add(id);
    stats[id ? "activeResolved" : "activeUnresolved"]++;
  }
  stats.greetings++;
  stats.titlesResolved += pending.available.length - unresolved.length;
  stats.titlesUnresolved += unresolved.length;

  const distinct = new Set(ids);
  const known = new Map(offer.available.map((quest) => [quest.id, quest]));
  offer.available = [...distinct].map((id) => known.get(id) ?? { id });
  offer.active = [...active];
  const shown = unresolved.filter((title) => index.isShared(context.locale, title));
  stats.titlesWithheld += unresolved.length - shown.length;
  if (shown.length > 0) offer.unresolvedTitles = shown;
  // Two entries resolving to one id means one of them is really another quest.
  offer.listComplete = !pending.incomplete && unresolved.length === 0 && distinct.size === ids.length && ids.length === pending.expected;
  if (offer.listComplete) stats.completeGreetings++;
}

function normalizeTitle(title: string): string {
  return title.trim();
}

function addTo(map: Map<string, Set<number>>, key: string, questId: number): void {
  const ids = map.get(key);
  if (ids) ids.add(questId);
  else map.set(key, new Set([questId]));
}

function only(ids: Set<number>): number {
  return ids.values().next().value as number;
}
