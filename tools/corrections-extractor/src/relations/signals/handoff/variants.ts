// Same-name quest variants: Forever often ships one copy of a quest per faction, race or
// profession under the same name, and a quest that needs "the" previous step accepts any copy
// (96 of 100 multi-quest preQuestSingle lists in the authored data are such copies). The traces
// only show the copies that observed characters took, so observed edges are extended to them.

import { classAllowed, isForeverQuestId, KNOWN_RACE_IDS, raceAllowed } from "../../core/eligibility";
import type { CatalogQuest, QuestEpisode } from "../../core/types";
import type { QuestLookup } from "./detect";

/** Quest id -> the other quest ids with exactly the same name. Quests without a name have none. */
export function sameNameVariants(quests: Iterable<CatalogQuest>): Map<number, number[]> {
  const byName = new Map<string, number[]>();
  for (const quest of quests) {
    const name = quest.name?.trim();
    if (!name) continue;
    const ids = byName.get(name);
    if (ids) ids.push(quest.id);
    else byName.set(name, [quest.id]);
  }
  const variants = new Map<number, number[]>();
  for (const ids of byName.values()) {
    if (ids.length < 2) continue;
    for (const id of ids) variants.set(id, ids.filter((other) => other !== id));
  }
  return variants;
}

/**
 * - prerequisite: P' -> X, another copy of the observed prerequisite.
 * - quest: P -> X', another copy of the observed quest.
 * - both: P' -> X', the observed edge in another copy of the whole step.
 */
export type VariantKind = "prerequisite" | "quest" | "both";

export interface VariantEdge {
  prerequisite: number;
  quest: number;
  kind: VariantKind;
  /** The observed edge this one was derived from. */
  observed: { prerequisite: number; quest: number };
}

const CLASS_IDS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];

/** Some character could take both quests: their race and class requirements overlap. */
export function couldShareCharacter(a: CatalogQuest | undefined, b: CatalogQuest | undefined): boolean {
  if (!a || !b) return false;
  const races = KNOWN_RACE_IDS.some((race) => raceAllowed(a.requiredRaces, race) && raceAllowed(b.requiredRaces, race));
  const classes = CLASS_IDS.some((id) => classAllowed(a.requiredClasses, id) && classAllowed(b.requiredClasses, id));
  return races && classes;
}

/**
 * Edges implied by observed edges P -> X through same-name copies, one per (edge, observed edge):
 * the same edge can follow from several observed ones. A copy only stands in when a
 * character could take it together with the other side, and copies are kept on the same side of
 * the Classic/Forever id boundary as the quest they replace: Forever reworked many Classic quests
 * under their old names, and the old copy usually leads somewhere else.
 */
export function variantEdges(observed: Iterable<{ prerequisite: number; quest: number }>, variants: Map<number, number[]>, lookup: QuestLookup): VariantEdge[] {
  const edges: VariantEdge[] = [];
  const add = (prerequisite: number, quest: number, kind: VariantKind, from: { prerequisite: number; quest: number }) => {
    if (couldShareCharacter(lookup(prerequisite), lookup(quest))) edges.push({ prerequisite, quest, kind, observed: from });
  };
  for (const edge of observed) {
    // P and X sharing a name are steps of one chain; their copies are other steps, not variants.
    const name = lookup(edge.prerequisite)?.name;
    if (name !== undefined && name === lookup(edge.quest)?.name) continue;
    const prerequisites = copiesOf(edge.prerequisite, variants);
    const quests = copiesOf(edge.quest, variants);
    for (const prerequisite of prerequisites) add(prerequisite, edge.quest, "prerequisite", edge);
    for (const quest of quests) add(edge.prerequisite, quest, "quest", edge);
    for (const prerequisite of prerequisites) for (const quest of quests) add(prerequisite, quest, "both", edge);
  }
  return edges;
}

function copiesOf(questId: number, variants: Map<number, number[]>): number[] {
  const forever = isForeverQuestId(questId);
  return (variants.get(questId) ?? []).filter((other) => isForeverQuestId(other) === forever);
}

/**
 * When each character first had the tracked quests flagged completed. Forever flags every
 * parallel copy of a quest at once when one is turned in, so in the traces true copies always
 * appear in the same completed-quest delta (1,830 of 1,830 authored same-name OR pairs), while
 * same-name steps of one chain, such as Classic's seven "The Defias Brotherhood" quests, complete
 * at separate turn-ins.
 */
/** Deltas adding more quests than this are catch-up reads (seen with 127-342 quests), not turn-ins. */
const MAX_TURN_IN_DELTA = 20;

export class CompletionMoments {
  /** characterKey -> quest id -> "episodeKey@t" of the delta that first added it. */
  private readonly byCharacter = new Map<string, Map<number, string>>();

  constructor(private readonly tracked: ReadonlySet<number>) {}

  add(episode: QuestEpisode): void {
    let moments = this.byCharacter.get(episode.characterKey);
    for (const change of episode.completed?.changes ?? []) {
      if ((change.add?.length ?? 0) > MAX_TURN_IN_DELTA) continue;
      for (const questId of change.add ?? []) {
        if (!this.tracked.has(questId)) continue;
        if (!moments) this.byCharacter.set(episode.characterKey, (moments = new Map()));
        if (!moments.has(questId)) moments.set(questId, `${episode.key}@${change.t}`);
      }
    }
  }

  /** Some character had both flagged in one delta, and none had them flagged separately. */
  alwaysTogether(a: number, b: number): boolean {
    let together = 0;
    for (const moments of this.byCharacter.values()) {
      const momentA = moments.get(a);
      const momentB = moments.get(b);
      if (momentA === undefined || momentB === undefined) continue;
      if (momentA !== momentB) return false;
      together++;
    }
    return together > 0;
  }
}

/** The quests a CompletionMoments needs to track for these edges. */
export function questsInVariantEdges(edges: Iterable<VariantEdge>): Set<number> {
  const ids = new Set<number>();
  for (const edge of edges) for (const id of [edge.prerequisite, edge.quest, edge.observed.prerequisite, edge.observed.quest]) ids.add(id);
  return ids;
}

/** Whether every copy in the edge was seen completing together with the quest it stands in for. */
export function isParallelCopy(edge: VariantEdge, moments: CompletionMoments): boolean {
  if (edge.kind !== "quest" && !moments.alwaysTogether(edge.prerequisite, edge.observed.prerequisite)) return false;
  if (edge.kind !== "prerequisite" && !moments.alwaysTogether(edge.quest, edge.observed.quest)) return false;
  return true;
}
