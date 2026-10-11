// Exclusion from co-flagging: completing either quest of a server exclusive group marks its
// siblings completed too, in the same instant.
//
// This is how most exclusiveTo groups actually show up in completion histories (measured on both
// ground-truth tiers: e.g. every character with one "Camping 101: Mining" copy has all of them,
// added in the same completed-set delta as the one it turned in). "Never together" is the textbook
// signature but is rare in practice; see exclusion.ts.
//
// An observation is one character whose completed set gained both quests within `windowSeconds`
// while it turned in at most one of them. A pair qualifies when enough characters show that,
// every character who completed either has both, at the same moment as far as can be seen, and
// nobody ever had both active at once (which exclusiveTo would have hidden).
// A breadcrumb flagged by its target's turn-in fails that check, because characters who took the
// breadcrumb itself completed it earlier than the target.

import type { Evidence } from "../../core/types";
import { heldTogether, type CharacterHistory, type Stamp } from "./history";
import type { Character, Population } from "./population";

export interface CoFlagParams {
  /** Completions this close together in one session count as one server-side flag. */
  windowSeconds: number;
  /** Characters observed gaining both at once while turning in at most one. */
  minObservations: number;
}

export interface CoFlagEdge {
  a: number;
  b: number;
  /** Characters seen gaining both at once while turning in at most one. */
  observations: number;
  /** Of those, characters that turned in `a` / `b` (the other was flagged). Both > 0 = symmetric. */
  turnedInA: number;
  turnedInB: number;
  /** Characters with both already completed when a session started (consistent, but no timing). */
  unordered: number;
  /** Characters with only one of them, or both at clearly different times. */
  contradict: number;
  evidence: Evidence[];
}

export interface CoFlagResult {
  edges: CoFlagEdge[];
  /**
   * Quests completed during a recorded session by at least minObservations characters: had they a
   * co-flagged sibling, it would have been seen landing with them.
   */
  covered: number[];
  /** Pairs with at least one observation. */
  observedPairs: number;
  /** Observed pairs dropped because some character completed one without the other. */
  contradicted: number;
}

export function inferCoFlags(population: Population, params: CoFlagParams): CoFlagResult {
  const observed = new Map<string, Observation>();
  for (const character of population.characters) {
    for (const [key, observation] of observationsOf(character, population, params.windowSeconds)) {
      const total = observed.get(key);
      if (!total) observed.set(key, observation);
      else {
        total.characters++;
        total.turnedInA += observation.turnedInA;
        total.turnedInB += observation.turnedInB;
        total.evidence.push(...observation.evidence.slice(0, Math.max(0, 3 - total.evidence.length)));
      }
    }
  }

  const covered = population.quests
    .filter((quest, index) => {
      const seenLanding = population.completers[index].filter((character) => !character.history.done.get(quest.id)!.atStart);
      return seenLanding.length >= params.minObservations;
    })
    .map((quest) => quest.id);
  const result: CoFlagResult = { edges: [], covered, observedPairs: observed.size, contradicted: 0 };
  for (const observation of observed.values()) {
    if (observation.characters < params.minObservations) continue;
    const check = checkPair(population, observation.a, observation.b, params.windowSeconds);
    if (check.contradict > 0) {
      result.contradicted++;
      continue;
    }
    result.edges.push({
      a: observation.a,
      b: observation.b,
      observations: observation.characters,
      turnedInA: observation.turnedInA,
      turnedInB: observation.turnedInB,
      unordered: check.unordered,
      contradict: check.contradict,
      evidence: observation.evidence,
    });
  }
  return result;
}

interface Observation {
  a: number;
  b: number;
  characters: number;
  turnedInA: number;
  turnedInB: number;
  evidence: Evidence[];
}

/** Pairs this character gained together, keyed "a:b" with a < b; one observation per pair. */
function observationsOf(character: Character, population: Population, windowSeconds: number): Map<string, Observation> {
  const history = character.history;
  const turnedIn = turnInsByEpisode(history);
  const byEpisode = new Map<string, Array<{ questId: number; t: number }>>();
  for (const index of character.done) {
    const questId = population.quests[index].id;
    const stamp = history.done.get(questId)!;
    if (stamp.atStart) continue;
    const list = byEpisode.get(stamp.episode);
    if (list) list.push({ questId, t: stamp.t });
    else byEpisode.set(stamp.episode, [{ questId, t: stamp.t }]);
  }

  const result = new Map<string, Observation>();
  for (const [episode, completions] of byEpisode) {
    completions.sort((x, y) => x.t - y.t);
    const handedIn = turnedIn.get(episode) ?? new Set<number>();
    for (let i = 0; i < completions.length; i++) {
      for (let j = i + 1; j < completions.length && completions[j].t - completions[i].t <= windowSeconds; j++) {
        const [first, second] = [completions[i], completions[j]];
        if (handedIn.has(first.questId) && handedIn.has(second.questId)) continue;
        const [a, b] = first.questId < second.questId ? [first.questId, second.questId] : [second.questId, first.questId];
        const key = `${a}:${b}`;
        if (result.has(key)) continue;
        const flagged = handedIn.has(a) ? b : handedIn.has(b) ? a : undefined;
        result.set(key, {
          a,
          b,
          characters: 1,
          turnedInA: handedIn.has(a) ? 1 : 0,
          turnedInB: handedIn.has(b) ? 1 : 0,
          evidence: [
            {
              ref: episode,
              t: first.t,
              text:
                `character ${history.key}: ${a} and ${b} became completed ${(second.t - first.t).toFixed(1)}s apart` +
                (flagged === undefined ? ", neither turned in" : `, only ${flagged === a ? b : a} turned in`),
            },
          ],
        });
      }
    }
  }
  return result;
}

function turnInsByEpisode(history: CharacterHistory): Map<string, Set<number>> {
  const result = new Map<string, Set<number>>();
  for (const timeline of history.timelines.values()) {
    const set = new Set<number>();
    for (const event of timeline.episode.questEvents) if (event.kind === "turnedIn") set.add(event.questId);
    result.set(timeline.episode.key, set);
  }
  return result;
}

/**
 * Every completer of either quest must have both, completed at the same moment where visible, and
 * no taker of both may have held them at once.
 */
function checkPair(population: Population, a: number, b: number, windowSeconds: number): { unordered: number; contradict: number } {
  const [indexA, indexB] = [population.indexOf.get(a)!, population.indexOf.get(b)!];
  const completers = new Set<Character>([...population.completers[indexA], ...population.completers[indexB]]);
  let unordered = 0;
  let contradict = 0;
  for (const character of completers) {
    const [doneA, doneB] = [character.history.done.get(a), character.history.done.get(b)];
    if (!doneA || !doneB) contradict++;
    else if (doneA.atStart && doneB.atStart && doneA.pos === doneB.pos) unordered++;
    else if (!sameMoment(doneA, doneB, windowSeconds)) contradict++;
  }
  for (const character of population.takers[indexA]) {
    if (character.history.taken.has(b) && heldTogether(character.history, a, b)) contradict++;
  }
  return { unordered, contradict };
}

function sameMoment(a: Stamp, b: Stamp, windowSeconds: number): boolean {
  return !a.atStart && !b.atStart && a.episode === b.episode && Math.abs(a.t - b.t) <= windowSeconds;
}
