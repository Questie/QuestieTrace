// Exclusion: X and Y are each commonly completed, yet no character ever had both.
//
// In practice this is the rare signature: most exclusive groups are co-flagged instead (see
// coflag.ts), and on the 2026-10 data no pair clears the thresholds below. It stays as the check
// for true choose-one quests, scored low until a human confirms an edge (calibration.ts).
//
// "Never together" only means something relative to how often they would meet by chance, so
// each pair gets the number of characters expected to complete both if completions were
// independent. That expectation is stratified by race/class cell and gated on level: two
// starting-zone quests of different races are never done together, but not because the server
// forbids it. A pair qualifies when the expectation is high enough that zero is surprising and
// no character ever held both at once (in the log together, or one in the log while the other
// was completed). Quests gated by skills, reputation or spells we cannot see are only compared with
// quests behind the same gate: "Camping 101: Alchemy" and "Camping 101: Mining" are never done
// together because few characters have both professions, not because the server forbids it.
//
// Choose-one-of-N groups (profession variants, reward choices) are rarely completed per member,
// so pairs inside one structural cluster (shared starter, or shared name stem) are judged
// together: every pair of a clique must be clean, and the clique's summed expectation must clear
// a lower bar than an unrelated pair needs.

import type { Evidence } from "../../core/types";
import { heldTogether, type CharacterHistory } from "./history";
import { couldTake, type Character, type Population } from "./population";

export interface ExclusionParams {
  /** Completers a quest needs before it takes part in pairwise tests. */
  minCompleters: number;
  /** Expected co-completions (under independence) an unrelated pair needs. */
  minExpected: number;
  /** Summed expected co-completions a structural clique needs. */
  minGroupExpected: number;
  /** Members of a structural clique need at least this many completers. */
  minGroupMemberCompleters: number;
}

export interface PairStats {
  a: number;
  b: number;
  /** Completers of a / b who could also have taken the other. */
  completersA: number;
  completersB: number;
  expected: number;
}

export interface ExclusionEdge extends PairStats {
  /** "pair": cleared minExpected alone. "group": member of a structural clique of `groupSize`. */
  kind: "pair" | "group";
  groupSize: number;
  groupExpected: number;
  /** Characters who took both at different times (abandoned one, then took the other). */
  switched: number;
  evidence: Evidence[];
}

export interface ExclusionResult {
  edges: ExclusionEdge[];
  testedPairs: number;
  /** Pairs that cleared the expectation but some character held both at once. */
  conflicted: number;
}

export function inferExclusions(population: Population, params: ExclusionParams): ExclusionResult {
  const stats = new PairCounter(population);
  const result: ExclusionResult = { edges: [], testedPairs: 0, conflicted: 0 };
  const seen = new Set<string>();
  const emit = (pair: PairStats, kind: ExclusionEdge["kind"], groupSize: number, groupExpected: number) => {
    const key = `${pair.a}:${pair.b}`;
    if (seen.has(key)) return;
    seen.add(key);
    const switched = stats.switched(population.indexOf.get(pair.a)!, population.indexOf.get(pair.b)!);
    result.edges.push({ ...pair, kind, groupSize, groupExpected, switched, evidence: evidenceFor(population, pair) });
  };

  const frequent = population.quests.map((_, index) => index).filter((index) => population.completers[index].length >= params.minCompleters);
  for (let i = 0; i < frequent.length; i++) {
    for (let j = i + 1; j < frequent.length; j++) {
      const [a, b] = [frequent[i], frequent[j]];
      // E <= min(completers), so small quests can never clear the bar alone.
      if (Math.min(population.completers[a].length, population.completers[b].length) < params.minExpected) continue;
      if (!sameHiddenGate(population, a, b) || stats.coCompleted(a, b)) continue;
      result.testedPairs++;
      const pair = stats.pair(a, b);
      if (pair.expected < params.minExpected) continue;
      if (stats.conflicted(a, b)) {
        result.conflicted++;
        continue;
      }
      emit(pair, "pair", 2, pair.expected);
    }
  }

  for (const cluster of structuralClusters(population, params.minGroupMemberCompleters)) {
    const clean = (a: number, b: number) =>
      sameHiddenGate(population, a, b) && !stats.coCompleted(a, b) && stats.pair(a, b).expected > 0 && !stats.conflicted(a, b);
    for (const clique of maximalCliques(cluster, clean)) {
      const pairs = clique.flatMap((a, i) => clique.slice(i + 1).map((b) => stats.pair(a, b)));
      const groupExpected = pairs.reduce((sum, pair) => sum + pair.expected, 0);
      if (groupExpected < params.minGroupExpected) continue;
      for (const pair of pairs) emit(pair, "group", clique.length, groupExpected);
    }
  }
  return result;
}

/** Both quests sit behind the same untracked requirement (skill, reputation, spell), or neither does. */
function sameHiddenGate(population: Population, a: number, b: number): boolean {
  const gate = (index: number) => {
    const quest = population.quests[index];
    return `${quest.requiredSkill?.[0]}:${quest.requiredMinRep?.[0]}:${quest.requiredMaxRep?.[0]}:${quest.requiredSpell}`;
  };
  return gate(a) === gate(b);
}

/** Pair statistics over the population, with the quest-index bitsets they need. */
class PairCounter {
  private readonly words: number;
  private readonly bits: Uint32Array[];
  private readonly byCell = new Map<string, Character[]>();
  private readonly pairs = new Map<number, PairStats>();

  constructor(private readonly population: Population) {
    this.words = Math.ceil(population.characters.length / 32);
    const position = new Map<Character, number>(population.characters.map((character, index) => [character, index]));
    this.bits = population.completers.map((completers) => {
      const set = new Uint32Array(this.words);
      for (const character of completers) {
        const index = position.get(character)!;
        set[index >>> 5] |= 1 << (index & 31);
      }
      return set;
    });
    for (const character of population.characters) {
      const list = this.byCell.get(character.cell.key);
      if (list) list.push(character);
      else this.byCell.set(character.cell.key, [character]);
    }
  }

  coCompleted(a: number, b: number): boolean {
    const [x, y] = [this.bits[a], this.bits[b]];
    for (let w = 0; w < this.words; w++) if ((x[w] & y[w]) !== 0) return true;
    return false;
  }

  /** Completers of each quest who could take the other, and the stratified expected overlap. */
  pair(a: number, b: number): PairStats {
    const key = Math.min(a, b) * this.population.quests.length + Math.max(a, b);
    const cached = this.pairs.get(key);
    if (cached) return cached;
    const stats = this.computePair(Math.min(a, b), Math.max(a, b));
    this.pairs.set(key, stats);
    return stats;
  }

  private computePair(a: number, b: number): PairStats {
    const { population } = this;
    const [questA, questB] = [population.quests[a], population.quests[b]];
    const perCell = new Map<string, { a: number; b: number }>();
    const count = (completers: Character[], other: number, side: "a" | "b") => {
      for (const character of completers) {
        if (!couldTake(character, other, population.quests[other])) continue;
        const entry = perCell.get(character.cell.key) ?? { a: 0, b: 0 };
        entry[side]++;
        perCell.set(character.cell.key, entry);
      }
    };
    count(population.completers[a], b, "a");
    count(population.completers[b], a, "b");

    let expected = 0;
    let completersA = 0;
    let completersB = 0;
    for (const [cellKey, counts] of perCell) {
      completersA += counts.a;
      completersB += counts.b;
      if (counts.a === 0 || counts.b === 0) continue;
      const eligible = this.byCell.get(cellKey)!.filter((character) => couldTake(character, a, questA) && couldTake(character, b, questB)).length;
      if (eligible > 0) expected += (counts.a * counts.b) / eligible;
    }
    return { a: questA.id, b: questB.id, completersA, completersB, expected };
  }

  /** Some character had both active at once: both in the log, or one in the log while the other was completed. */
  conflicted(a: number, b: number): boolean {
    return this.bothTaken(a, b).some((history) => heldTogether(history, this.population.quests[a].id, this.population.quests[b].id));
  }

  switched(a: number, b: number): number {
    const [idA, idB] = [this.population.quests[a].id, this.population.quests[b].id];
    return this.bothTaken(a, b).filter((history) => !heldTogether(history, idA, idB)).length;
  }

  private bothTaken(a: number, b: number): CharacterHistory[] {
    const idB = this.population.quests[b].id;
    return this.population.takers[a].filter((character) => character.history.taken.has(idB)).map((character) => character.history);
  }
}

/**
 * Quest-index clusters that plausibly form one choice: quests sharing a starter, or sharing a name
 * stem ("Camping 101: Alchemy" and "Camping 101: Herbalism"). Only members with enough completers.
 */
function structuralClusters(population: Population, minCompleters: number): number[][] {
  const groups = new Map<string, number[]>();
  const add = (key: string, index: number) => {
    const list = groups.get(key);
    if (list) list.push(index);
    else groups.set(key, [index]);
  };
  population.quests.forEach((quest, index) => {
    if (population.completers[index].length < minCompleters) return;
    for (const id of quest.starters.npcs) add(`npc:${id}`, index);
    for (const id of quest.starters.objects) add(`object:${id}`, index);
    for (const id of quest.starters.items) add(`item:${id}`, index);
    const stem = nameStem(quest.name);
    if (stem) add(`stem:${stem}`, index);
    if (quest.name) add(`name:${quest.name.toLowerCase()}`, index);
  });
  return [...groups.values()].filter((members) => members.length >= 2);
}

/** "Camping 101: Alchemy" -> "camping 101". Names without a separator are their own stem. */
export function nameStem(name: string | undefined): string | undefined {
  if (!name) return undefined;
  const stem = name.split(/:| - /)[0].trim().toLowerCase();
  return stem.length >= 4 ? stem : undefined;
}

/** Bron-Kerbosch over a small vertex set. Cliques of one are not returned. */
function maximalCliques(vertices: number[], adjacent: (a: number, b: number) => boolean): number[][] {
  const cliques: number[][] = [];
  const expand = (clique: number[], candidates: number[], excluded: number[]) => {
    if (candidates.length === 0 && excluded.length === 0) {
      if (clique.length >= 2) cliques.push(clique);
      return;
    }
    for (const vertex of [...candidates]) {
      const neighbours = (list: number[]) => list.filter((other) => other !== vertex && adjacent(vertex, other));
      expand([...clique, vertex], neighbours(candidates), neighbours(excluded));
      candidates = candidates.filter((other) => other !== vertex);
      excluded = [...excluded, vertex];
    }
  };
  expand([], vertices, []);
  return cliques;
}

/** Up to three completers per side who could have taken the other quest and never did. */
function evidenceFor(population: Population, pair: PairStats): Evidence[] {
  const side = (completed: number, other: number): Evidence[] => {
    const otherIndex = population.indexOf.get(other)!;
    return population.completers[population.indexOf.get(completed)!]
      .filter((character) => couldTake(character, otherIndex, population.quests[otherIndex]) && !character.history.taken.has(other))
      .slice(0, 3)
      .map(({ history }) => ({
        ref: history.done.get(completed)!.episode,
        text:
          `character ${history.key} (race ${history.player.raceId}, class ${history.player.classId}, level ${history.maxLevel}): ` +
          `completed ${completed}, never took ${other}`,
      }));
  };
  return [...side(pair.a, pair.b), ...side(pair.b, pair.a)];
}
