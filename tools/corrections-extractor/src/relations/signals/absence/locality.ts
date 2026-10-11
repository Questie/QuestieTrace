// Which suspects are plausible relationship partners for a blocked quest, before any evidence.
//
// Questie's relations are local: exclusiveTo partners are nearly always same-name variants
// (zone, race or faction copies), chains and breadcrumbs hand off at the next quest's giver,
// prerequisites are turned in where the next quest starts. A blocked moment of a veteran
// character can have hundreds of consistent suspects (every quest they completed that no
// offered character had); weighting unrelated ones down lets the few plausible ones carry
// the blame, while a sharp moment (one changed quest) still convicts an unrelated suspect.

import type { QuestCatalog } from "../../core/types";
import { pairKey, type OfferIndex } from "./offers";

export type LocalReason = "sameName" | "listedTogether" | "sameGiver" | "handoff" | "finisherIsGiver";

export class Locality {
  private readonly giversOf = new Map<number, Set<string>>();
  private readonly finishersOf = new Map<number, Set<string>>();

  constructor(
    private readonly index: OfferIndex,
    private readonly catalog: QuestCatalog,
  ) {
    for (const [giver, quests] of index.questsByGiver) for (const questId of quests) addTo(this.giversOf, questId, giver);
    for (const quest of Object.values(catalog.quests)) {
      for (const id of quest.starters.npcs) addTo(this.giversOf, quest.id, `npc:${id}`);
      for (const id of quest.starters.objects) addTo(this.giversOf, quest.id, `object:${id}`);
      for (const id of quest.finishers.npcs) addTo(this.finishersOf, quest.id, `npc:${id}`);
      for (const id of quest.finishers.objects) addTo(this.finishersOf, quest.id, `object:${id}`);
    }
    for (const [questId, givers] of index.finishersOf) for (const giver of givers) addTo(this.finishersOf, questId, giver);
  }

  /** The first reason the two quests are plausibly related, or undefined. */
  reason(a: number, b: number): LocalReason | undefined {
    const nameA = this.catalog.quests[a]?.name;
    if (nameA !== undefined && nameA === this.catalog.quests[b]?.name) return "sameName";
    if (this.index.coOffered.has(pairKey(a, b))) return "listedTogether";
    if (this.index.handoffs.has(`${a}>${b}`) || this.index.handoffs.has(`${b}>${a}`)) return "handoff";
    if (intersects(this.giversOf.get(a), this.giversOf.get(b))) return "sameGiver";
    if (intersects(this.finishersOf.get(a), this.giversOf.get(b)) || intersects(this.finishersOf.get(b), this.giversOf.get(a))) return "finisherIsGiver";
    return undefined;
  }
}

function intersects(a: Set<string> | undefined, b: Set<string> | undefined): boolean {
  if (!a || !b) return false;
  for (const value of a) if (b.has(value)) return true;
  return false;
}

function addTo<K, V>(map: Map<K, Set<V>>, key: K, value: V): void {
  const set = map.get(key);
  if (set) set.add(value);
  else map.set(key, new Set([value]));
}
