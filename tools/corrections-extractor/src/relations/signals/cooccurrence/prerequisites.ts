// Prerequisites from implication: every character that took X had already completed P.
//
// A taker of X (held, accepted or completed it) who had not completed P at that moment
// contradicts "P is a prerequisite of X"; so does one who never completed P at all. Survivors
// are then filtered on:
// - support: enough eligible takers, some of them with P observed strictly before X;
// - lift: P must not be something nearly every comparable character completes anyway
//   (nullProbability = chance that all takers had P if X and P were independent);
// - nearness: if X => P2 => P1, only P2 is X's direct prerequisite. Zone-mates done earlier by
//   every taker survive that pruning, so the remaining steps are ranked by hand-offs (taking X
//   within minutes of completing P), which is what tells a direct prerequisite apart.
// Co-flagged copies (coflag.ts) form one step: any of them will do, and they are never each
// other's prerequisite. A P that only some takers could take (race/class gated) is a variant: it
// is emitted only when such variants together cover every taker, as alternatives (P1 OR P2).

import type { CatalogQuest, Evidence, RelationField } from "../../core/types";
import { order, sameSession, type Stamp } from "./history";
import { couldTake, type Cell, type Character, type Population } from "./population";

export interface PrerequisiteParams {
  /** Takers of X (eligible for P) needed before an edge is judged at all. */
  minTakers: number;
  /** Takers with P completed strictly before they took X. */
  minOrdered: number;
  /** Largest acceptable chance that all takers had P although X and P are unrelated. */
  maxNullProbability: number;
  orderToleranceSeconds: number;
  /** Took X at most this long after completing P, in the same session: a hand-off. */
  handoffSeconds: number;
}

/** P survived the implication test for X. */
export interface Implication {
  questId: number;
  prereq: number;
  /** Takers of X whose race/class allow P. */
  takers: number;
  ordered: number;
  unordered: number;
  /**
   * Seconds from completing P to taking X, for ordered takers who did both in one session, except
   * those whose P turn-in levelled them up to X's required level: that unlocks X without any
   * relation (a class quest at level 10 is often taken right after whichever quest gave level 10).
   */
  gaps: number[];
  /** Same-session ordered takers who reached X's required level in between. */
  levelUps: number;
  /** Cells (race:class) of the takers that could take P. */
  cells: Set<string>;
  /** The ordered takers that took X soonest after completing P. */
  witnesses: Evidence[];
}

export interface PrerequisiteEdge extends Implication {
  field: RelationField;
  allTakers: number;
  /** Comparable characters who did not take X: how many could take P, and how many completed it. */
  baseline: { completed: number; of: number };
  nullProbability: number;
  /** "full": every taker could take P. "variant": one of several race/class alternatives. */
  shape: "full" | "variant";
  /** Steps emitted for X (an AND group of 1 is preQuestSingle); co-flagged copies are one step. */
  siblings: number;
  /** Takers who took X within handoffSeconds of completing P. */
  handoffs: number;
  /** 1 = the step taken soonest after completion; see rankSteps. */
  rank: number;
}

export interface PrerequisiteResult {
  edges: PrerequisiteEdge[];
  /** Quests with enough takers to be judged. */
  covered: number[];
  /** Accepted before pruning, then dropped because a nearer prerequisite implies them. */
  pruned: number;
  /** Variants whose alternatives did not cover every taker. */
  uncoveredVariants: number;
}

/** Whether two quests are copies the server completes together (see coflag.ts). */
export type CoFlagged = (a: number, b: number) => boolean;

export function inferPrerequisites(population: Population, params: PrerequisiteParams, coFlagged: CoFlagged = () => false): PrerequisiteResult {
  const finder = new ImplicationFinder(population, params);
  const result: PrerequisiteResult = { edges: [], covered: [], pruned: 0, uncoveredVariants: 0 };

  population.quests.forEach((quest, questIndex) => {
    const takers = population.takers[questIndex];
    if (takers.length < params.minTakers) return;
    result.covered.push(quest.id);

    const accepted: PrerequisiteEdge[] = [];
    for (const implication of finder.survivors(questIndex).values()) {
      if (implication.takers < params.minTakers || implication.ordered < params.minOrdered) continue;
      // A co-flagged copy completes a moment before the flag lands on X, which reads as a hand-off.
      if (coFlagged(quest.id, implication.prereq)) continue;
      const baseline = baselineFor(population, questIndex, implication.prereq);
      const nullProbability = smoothedRate(baseline) ** implication.takers;
      if (nullProbability > params.maxNullProbability) continue;
      accepted.push({
        ...implication,
        field: "preQuestSingle",
        allTakers: takers.length,
        baseline,
        nullProbability,
        shape: implication.takers === takers.length ? "full" : "variant",
        siblings: 0,
        handoffs: implication.gaps.filter((gap) => gap <= params.handoffSeconds).length,
        rank: 0,
      });
    }

    // P1 is pruned when nearer steps implying it cover every race/class that needs P1: an
    // Alliance-only P2 => P1 says nothing about Horde takers. Co-flagged copies seem to imply each
    // other (the turned-in copy is stamped a moment before the flagged ones), so they never count.
    const nearest = accepted.filter((edge) => {
      const impliers = accepted.filter((other) => other !== edge && !coFlagged(other.prereq, edge.prereq) && finder.implies(other.prereq, edge.prereq));
      const covered = new Set(impliers.flatMap((other) => [...other.cells]));
      return impliers.length === 0 || ![...edge.cells].every((cell) => covered.has(cell));
    });
    result.pruned += accepted.length - nearest.length;

    // One logical prerequisite is a group of co-flagged copies: completing any of them will do.
    const allCells = new Set(takers.map((taker) => taker.cell.key));
    const coversAll = (edges: PrerequisiteEdge[]) => {
      const cells = new Set(edges.flatMap((edge) => [...edge.cells]));
      return [...allCells].every((cell) => cells.has(cell));
    };
    const steps = groupCoFlagged(nearest, coFlagged);
    let emitted = steps.filter(coversAll);
    if (emitted.length === 0 && steps.length > 0) {
      // Race/class variants of one step (X needs P1 OR P2) must cover every taker between them.
      if (coversAll(steps.flat())) emitted = [steps.flat()];
      else result.uncoveredVariants += steps.flat().length;
    }
    // The step taken soonest after completion is the prerequisite (preQuestSingle, its copies and
    // variants as alternatives). Further steps that do not imply each other would make an AND
    // group; they are emitted as preQuestGroup, but are mostly zone-mates and score low.
    rankSteps(emitted).forEach((step, index) => {
      const field: RelationField = index === 0 ? "preQuestSingle" : "preQuestGroup";
      for (const edge of [...step].sort((a, b) => a.prereq - b.prereq)) {
        result.edges.push({ ...edge, field, siblings: emitted.length, rank: index + 1 });
      }
    });
  });
  return result;
}

function groupCoFlagged(edges: PrerequisiteEdge[], coFlagged: CoFlagged): PrerequisiteEdge[][] {
  const groups: PrerequisiteEdge[][] = [];
  for (const edge of edges) {
    const matching = groups.filter((group) => group.some((other) => coFlagged(other.prereq, edge.prereq)));
    const merged = [edge, ...matching.flat()];
    for (const group of matching) groups.splice(groups.indexOf(group), 1);
    groups.push(merged);
  }
  return groups;
}

/**
 * Orders steps so the likeliest direct prerequisite comes first, by each step's best member: most
 * hand-offs per taker, then the shortest median lead time, then the most ordered observations. A
 * zone-mate every taker happened to finish earlier has long, scattered lead times.
 */
function rankSteps(steps: PrerequisiteEdge[][]): PrerequisiteEdge[][] {
  const median = (values: number[]) => (values.length === 0 ? Infinity : [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]);
  const better = (a: PrerequisiteEdge, b: PrerequisiteEdge) =>
    b.handoffs / b.takers - a.handoffs / a.takers || median(a.gaps) - median(b.gaps) || b.ordered - a.ordered || a.prereq - b.prereq;
  const best = (step: PrerequisiteEdge[]) => [...step].sort(better)[0];
  return [...steps].sort((a, b) => better(best(a), best(b)));
}

function smoothedRate({ completed, of }: { completed: number; of: number }): number {
  return (completed + 1) / (of + 2);
}

/**
 * How common P is among characters at comparable progress who did not take X, taking the more
 * common of two populations: those who could take X (the right comparison, but class-gated
 * quests leave only a handful) and everyone who could take P at X's level. Overstating how common
 * P is only costs recall.
 */
function baselineFor(population: Population, questIndex: number, prereq: number): { completed: number; of: number } {
  const quest = population.quests[questIndex];
  const prereqIndex = population.indexOf.get(prereq)!;
  const narrow = { completed: 0, of: 0 };
  const broad = { completed: 0, of: 0 };
  for (const character of population.characters) {
    if (character.history.taken.has(quest.id) || !character.cell.allows[prereqIndex]) continue;
    const level = character.history.maxLevel;
    if (quest.requiredLevel !== undefined && level !== undefined && level < quest.requiredLevel) continue;
    const completed = character.history.done.has(prereq) ? 1 : 0;
    broad.of++;
    broad.completed += completed;
    if (couldTake(character, questIndex, quest)) {
      narrow.of++;
      narrow.completed += completed;
    }
  }
  return smoothedRate(narrow) >= smoothedRate(broad) ? narrow : broad;
}

/** Computes (and caches) the implications surviving for each quest. */
class ImplicationFinder {
  private readonly cache = new Map<number, Map<number, Implication>>();
  private readonly counts: Int32Array;

  constructor(
    private readonly population: Population,
    private readonly params: PrerequisiteParams,
  ) {
    this.counts = new Int32Array(population.quests.length);
  }

  /** `later` => `earlier` with at least one character showing the order (used to prune transitive edges). */
  implies(later: number, earlier: number): boolean {
    const index = this.population.indexOf.get(later);
    if (index === undefined) return false;
    return (this.survivors(index).get(earlier)?.ordered ?? 0) > 0;
  }

  survivors(questIndex: number): Map<number, Implication> {
    const cached = this.cache.get(questIndex);
    if (cached) return cached;

    const { population, params, counts } = this;
    const questId = population.quests[questIndex].id;
    const takers = population.takers[questIndex];
    const cells = new Map<Cell, Character[]>();
    for (const taker of takers) {
      const list = cells.get(taker.cell);
      if (list) list.push(taker);
      else cells.set(taker.cell, [taker]);
    }

    const touched: number[] = [];
    for (const taker of takers) {
      for (const index of taker.done) {
        if (index === questIndex || !taker.cell.allows[index]) continue;
        if (counts[index]++ === 0) touched.push(index);
      }
    }

    const result = new Map<number, Implication>();
    for (const index of touched) {
      const done = counts[index];
      counts[index] = 0;
      let eligible = 0;
      for (const [cell, members] of cells) if (cell.allows[index]) eligible += members.length;
      // An eligible taker who never completed P contradicts it; measured, tolerating even 2% of
      // takers lets more zone-mates through than it rescues prerequisites.
      if (done < eligible) continue;
      const implication = this.checkOrder(population.quests[questIndex], population.quests[index].id, index, cells);
      if (implication) result.set(implication.prereq, implication);
    }
    this.cache.set(questIndex, result);
    return result;
  }

  /** Undefined as soon as one taker took X before completing P. */
  private checkOrder(quest: CatalogQuest, prereq: number, prereqIndex: number, cells: Map<Cell, Character[]>): Implication | undefined {
    const questId = quest.id;
    const implication: Implication = { questId, prereq, takers: 0, ordered: 0, unordered: 0, gaps: [], levelUps: 0, cells: new Set(), witnesses: [] };
    const witnesses: Array<{ lead: number; character: Character; done: Stamp; taken: Stamp }> = [];
    for (const [cell, members] of cells) {
      if (!cell.allows[prereqIndex]) continue;
      implication.cells.add(cell.key);
      for (const character of members) {
        implication.takers++;
        const taken = character.history.taken.get(questId)!;
        const done = character.history.done.get(prereq)!;
        const relation = order(done, taken, this.params.orderToleranceSeconds);
        if (relation === "after") return undefined;
        if (relation === "unordered") {
          implication.unordered++;
          continue;
        }
        implication.ordered++;
        const lead = sameSession(done, taken) ? taken.t - done.t : Infinity;
        if (lead !== Infinity && reachedLevel(character, done, taken, quest.requiredLevel)) implication.levelUps++;
        else if (lead !== Infinity) implication.gaps.push(lead);
        witnesses.push({ lead, character, done, taken });
      }
    }
    implication.witnesses = witnesses
      .sort((a, b) => a.lead - b.lead)
      .slice(0, 5)
      .map((entry) => witness(entry.character, prereq, questId, entry.done, entry.taken));
    return implication;
  }
}

const LEVEL_UP_LEAD_SECONDS = 5;

/** The character went from below `level` to at least `level` around `from` and before `to`, in one session. */
function reachedLevel(character: Character, from: Stamp, to: Stamp, level: number | undefined): boolean {
  const timeline = character.history.timelines.get(from.episode);
  if (level === undefined || !timeline) return false;
  // The level-up is often logged a moment before the turn-in event that caused it.
  const [before, after] = [timeline.levelAt(from.t - LEVEL_UP_LEAD_SECONDS), timeline.levelAt(to.t)];
  return before !== undefined && after !== undefined && before < level && after >= level;
}

function describe(stamp: Stamp): string {
  return stamp.atStart ? `before session ${stamp.episode} started` : `at ${stamp.episode} t=${stamp.t.toFixed(1)}`;
}

function witness(character: Character, prereq: number, questId: number, done: Stamp, taken: Stamp): Evidence {
  return {
    ref: taken.episode,
    t: taken.atStart ? undefined : taken.t,
    text: `character ${character.history.key}: completed ${prereq} ${describe(done)}, took ${questId} ${describe(taken)}`,
  };
}
