// Faction/race/class variants: same-name copies of a quest that no single character can both
// take. Forever-new prerequisite lists with several entries are almost always these ("the
// Alliance or the Horde copy"), so they are how an OR prerequisite is recognised.

import { classAllowed, KNOWN_RACE_IDS, raceAllowed } from "../../core/eligibility";
import type { CatalogQuest, QuestCatalog } from "../../core/types";

const MAX_CLASS_ID = 16;
const CLASS_IDS = Array.from({ length: MAX_CLASS_ID }, (_, index) => index + 1);

type Allows = (mask: number | undefined, id: number) => boolean;

/** Ids a mask allows, out of `universe` (an empty or 0 mask allows all of it). */
function allowedIds(mask: number | undefined, universe: ReadonlySet<number>, allows: Allows): Set<number> {
  const ids = new Set<number>();
  for (const id of universe) if (allows(mask, id)) ids.add(id);
  return ids;
}

/**
 * Ids some quest in the catalog explicitly allows: the playable races/classes, not every known id.
 * Falls back to every id when no quest restricts at all (otherwise every audience would be empty).
 */
function idsInUse(quests: CatalogQuest[], maskOf: (quest: CatalogQuest) => number | undefined, ids: readonly number[], allows: Allows): Set<number> {
  const used = new Set<number>();
  for (const quest of quests) {
    const mask = maskOf(quest);
    if (!mask) continue;
    for (const id of ids) if (allows(mask, id)) used.add(id);
  }
  return used.size > 0 ? used : new Set(ids);
}

function intersects(a: ReadonlySet<number>, b: ReadonlySet<number>): boolean {
  for (const id of a) if (b.has(id)) return true;
  return false;
}

export class VariantIndex {
  private readonly byName = new Map<string, CatalogQuest[]>();
  private readonly races = new Map<number, Set<number>>();
  private readonly classes = new Map<number, Set<number>>();
  private readonly raceUniverse: Set<number>;
  private readonly classUniverse: Set<number>;

  constructor(private readonly catalog: QuestCatalog) {
    const quests = Object.values(catalog.quests);
    this.raceUniverse = idsInUse(quests, (quest) => quest.requiredRaces, KNOWN_RACE_IDS, raceAllowed);
    this.classUniverse = idsInUse(quests, (quest) => quest.requiredClasses, CLASS_IDS, classAllowed);
    for (const quest of quests) {
      if (!quest.name) continue;
      const list = this.byName.get(quest.name);
      if (list) list.push(quest);
      else this.byName.set(quest.name, [quest]);
    }
  }

  nameOf(questId: number): string | undefined {
    return this.catalog.quests[questId]?.name;
  }

  racesOf(questId: number): Set<number> {
    let set = this.races.get(questId);
    if (!set) this.races.set(questId, (set = allowedIds(this.catalog.quests[questId]?.requiredRaces, this.raceUniverse, raceAllowed)));
    return set;
  }

  classesOf(questId: number): Set<number> {
    let set = this.classes.get(questId);
    if (!set) this.classes.set(questId, (set = allowedIds(this.catalog.quests[questId]?.requiredClasses, this.classUniverse, classAllowed)));
    return set;
  }

  /** True when no character can take both quests (disjoint races or disjoint classes). */
  exclusiveAudiences(a: number, b: number): boolean {
    return !intersects(this.racesOf(a), this.racesOf(b)) || !intersects(this.classesOf(a), this.classesOf(b));
  }

  /** Same-name quests whose audience does not overlap `questId`'s, ascending. */
  variantsOf(questId: number): number[] {
    const name = this.catalog.quests[questId]?.name;
    if (!name) return [];
    return (this.byName.get(name) ?? [])
      .filter((quest) => quest.id !== questId && this.exclusiveAudiences(questId, quest.id))
      .map((quest) => quest.id)
      .sort((a, b) => a - b);
  }

  /**
   * Races allowed to take `questId` that none of `prerequisites` allows (class gaps are ignored:
   * class quests rarely have class-specific prerequisites for other classes). A non-empty result
   * means a preQuestSingle of exactly these quests would hide `questId` from those races.
   */
  uncoveredRaces(questId: number, prerequisites: readonly number[]): number[] {
    const covered = new Set<number>();
    for (const prerequisite of prerequisites) for (const race of this.racesOf(prerequisite)) covered.add(race);
    return [...this.racesOf(questId)].filter((race) => !covered.has(race));
  }
}
