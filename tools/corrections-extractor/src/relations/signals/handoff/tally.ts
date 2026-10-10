// Folds per-episode observations into per-pair counts of independent characters, then into
// candidates. One character contributes at most once to each count of a pair, however many
// sessions or repeats it has.

import type { Evidence, QuestEpisode } from "../../core/types";
import type { CounterKind, CounterObservation } from "./counter";
import { giverLabel, type HandoffObservation } from "./detect";

/** Per (P, X) pair: the characters behind each kind of observation. */
export interface PairStats {
  /** P: the quest turned in. */
  prerequisite: number;
  /** X: the quest offered afterwards. */
  quest: number;
  pop: Set<string>;
  popConfounded: Set<string>;
  list: Set<string>;
  listConfounded: Set<string>;
  notStarted: Set<string>;
  inLog: Set<string>;
  alternative: Set<string>;
  /**
   * For each other observed prerequisite Q of X: the characters for whom Q was already completed,
   * or not, when they turned in P and got X. Tells ALL-of groups from ANY-of lists.
   */
  companions: Map<number, { done: Set<string>; notDone: Set<string> }>;
  /** Supporting examples; lower priority sorts first (clean pops, then clean lists, ...). */
  evidence: Array<{ priority: number; evidence: Evidence }>;
  counterEvidence: Evidence[];
}

const MAX_EVIDENCE = 5;
const MAX_COUNTER_EVIDENCE = 2;

export function pairKey(prerequisite: number, quest: number): string {
  return `${prerequisite}:${quest}`;
}

export class HandoffTally {
  readonly pairs = new Map<string, PairStats>();

  /** The pair's stats, created empty if needed (variant edges are tracked for counter-evidence only). */
  statsFor(prerequisite: number, quest: number): PairStats {
    const key = pairKey(prerequisite, quest);
    let stats = this.pairs.get(key);
    if (!stats) {
      stats = {
        prerequisite,
        quest,
        pop: new Set(),
        popConfounded: new Set(),
        list: new Set(),
        listConfounded: new Set(),
        notStarted: new Set(),
        inLog: new Set(),
        alternative: new Set(),
        companions: new Map(),
        evidence: [],
        counterEvidence: [],
      };
      this.pairs.set(key, stats);
    }
    return stats;
  }

  addHandoff(episode: QuestEpisode, observation: HandoffObservation): void {
    const stats = this.statsFor(observation.turnedIn, observation.offered);
    const clean = observation.confounders.length === 0;
    const bucket = observation.kind === "pop" ? (clean ? stats.pop : stats.popConfounded) : clean ? stats.list : stats.listConfounded;
    if (bucket.has(episode.characterKey)) return;
    bucket.add(episode.characterKey);
    const priority = (observation.kind === "pop" ? 0 : 2) + (clean ? 0 : 1);
    stats.evidence.push({ priority, evidence: { ref: episode.key, t: observation.turnInAt, text: describeHandoff(observation) } });
  }

  addCompanionStates(episode: QuestEpisode, observation: HandoffObservation, states: Iterable<[questId: number, completed: boolean]>): void {
    const stats = this.pairs.get(pairKey(observation.turnedIn, observation.offered));
    if (!stats) return;
    for (const [questId, completed] of states) {
      let entry = stats.companions.get(questId);
      if (!entry) stats.companions.set(questId, (entry = { done: new Set(), notDone: new Set() }));
      (completed ? entry.done : entry.notDone).add(episode.characterKey);
    }
  }

  addCounter(episode: QuestEpisode, observation: CounterObservation): void {
    const stats = this.pairs.get(pairKey(observation.prerequisite, observation.quest));
    if (!stats) return;
    const bucket = COUNTER_BUCKET[observation.kind](stats);
    if (bucket.has(episode.characterKey)) return;
    bucket.add(episode.characterKey);
    if (observation.kind !== "alternative") stats.counterEvidence.push({ ref: episode.key, t: observation.t, text: describeCounter(observation) });
  }
}

const COUNTER_BUCKET: Record<CounterKind, (stats: PairStats) => Set<string>> = {
  notStarted: (stats) => stats.notStarted,
  inLog: (stats) => stats.inLog,
  alternative: (stats) => stats.alternative,
};

function describeHandoff(observation: HandoffObservation): string {
  const { turnedIn, offered, giver, turnInAt, offeredAt } = observation;
  const delay = (offeredAt - turnInAt).toFixed(1);
  const base =
    observation.kind === "pop"
      ? `turned in ${turnedIn} at ${giverLabel(giver)}, its accept dialog for ${offered} opened ${delay}s later`
      : `turned in ${turnedIn} at ${giverLabel(giver)}; its list ${delay}s later offered ${offered}, its list ${(turnInAt - observation.previousListAt!).toFixed(1)}s before did not`;
  return observation.confounders.length > 0 ? `${base} (also: ${observation.confounders.slice(0, 3).join(", ")})` : base;
}

function describeCounter(observation: CounterObservation): string {
  const where = observation.giver ? ` by ${giverLabel(observation.giver)}` : "";
  const state = observation.kind === "inLog" ? "in the log, not completed" : "not started";
  return `against: offered ${observation.quest}${where} while ${observation.prerequisite} was ${state}`;
}

/** Up to MAX_EVIDENCE examples, pops first, keeping room for a couple of counter-examples. */
export function pickEvidence(stats: PairStats): Evidence[] {
  const support = [...stats.evidence].sort((a, b) => a.priority - b.priority).map((entry) => entry.evidence);
  const counter = stats.counterEvidence.slice(0, MAX_COUNTER_EVIDENCE);
  return [...support.slice(0, MAX_EVIDENCE - counter.length), ...counter];
}

export function countsNote(stats: PairStats): string {
  return (
    `pop ${stats.pop.size}+${stats.popConfounded.size}?, list ${stats.list.size}+${stats.listConfounded.size}?, ` +
    `against ${stats.notStarted.size} not started / ${stats.inLog.size} in log, ${stats.alternative.size} via alternatives`
  );
}

