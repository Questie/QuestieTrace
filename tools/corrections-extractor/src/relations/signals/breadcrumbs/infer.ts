// Turns per-episode observations into scored candidates: each kind of evidence is counted per
// independent character, a pair must be both optional and dependent (see observe.ts), and each
// breadcrumb keeps only its best target because breadcrumbForQuestId is a single quest.

import type { CatalogQuest, Evidence, QuestCatalog, QuestEpisode, RelationCandidate } from "../../core/types";
import { DEFAULT_PARAMS, LinkCollector, pairKey, type BreadcrumbParams } from "./links";
import { observeEpisode, type Observation, type ObservationKind } from "./observe";

// IsBreadcrumbQuest(B) says B is optional, not which target it leads to, so the flag stands in for
// "free" characters and never for the pair-specific dependent kinds.
const OPTIONAL_KINDS: readonly ObservationKind[] = ["free", "flagged"];
const DEPENDENT_KINDS: readonly ObservationKind[] = ["serverDrop", "reveal", "autoPop", "blockB"];
const CONTRA_KINDS: readonly ObservationKind[] = ["unlock", "showB", "showT", "notFlagged"];
const EXAMPLE_ORDER: readonly ObservationKind[] = [...OPTIONAL_KINDS, ...DEPENDENT_KINDS, ...CONTRA_KINDS];

// Weights are calibrated against Questie's Classic and Forever breadcrumbs (score command), so a
// score roughly reads as the chance the edge is right.

/**
 * How strongly one character's observation of each dependent kind says B and T gate each other.
 * The server dropping B as T is accepted is the breadcrumb mechanism itself. Chains also auto-pop
 * T or reveal it on B's turn-in, but optionality already rules chains out. B missing once T is done
 * is weakest, because B's own prerequisites hide it just as well.
 */
const DEPENDENT_WEIGHT: Partial<Record<ObservationKind, number>> = {
  serverDrop: 0.95,
  reveal: 0.8,
  autoPop: 0.75,
  blockB: 0.5,
};

/**
 * Characters offered B while T was done, or T while B was in the log. Real breadcrumbs almost never
 * show either (Forever blocks both ways), so one is treated as noise and more rule the pair out.
 */
const MAX_SHOWN = 1;
const SHOWN_PENALTY = 0.25;

/** One character's "removed" could still be a manual abandon ingest could not tell apart. */
const MIN_SERVER_DROP_CHARACTERS = 2;

export interface PairTally {
  breadcrumb: number;
  target: number;
  /** Characters per observation kind. */
  characters: Map<ObservationKind, Set<string>>;
  examples: Map<ObservationKind, Evidence[]>;
}

export class Tally {
  readonly pairs = new Map<string, PairTally>();

  add(characterKey: string, observations: readonly Observation[]): void {
    for (const { breadcrumb, target, kind, evidence } of observations) {
      const key = pairKey(breadcrumb, target);
      let pair = this.pairs.get(key);
      if (!pair) {
        pair = { breadcrumb, target, characters: new Map(), examples: new Map() };
        this.pairs.set(key, pair);
      }
      const characters = pair.characters.get(kind) ?? new Set<string>();
      if (!characters.has(characterKey)) {
        characters.add(characterKey);
        pair.characters.set(kind, characters);
        const examples = pair.examples.get(kind) ?? [];
        if (examples.length < 2) pair.examples.set(kind, [...examples, evidence]);
      }
    }
  }
}

function count(pair: PairTally, kind: ObservationKind): number {
  return pair.characters.get(kind)?.size ?? 0;
}

function charactersOf(pair: PairTally, kinds: readonly ObservationKind[]): Set<string> {
  const result = new Set<string>();
  for (const kind of kinds) for (const character of pair.characters.get(kind) ?? []) result.add(character);
  return result;
}

export interface PairJudgement {
  breadcrumb: number;
  target: number;
  score: number;
  support: number;
  contradict: number;
  counts: Partial<Record<ObservationKind, number>>;
  evidence: Evidence[];
}

/** Undefined unless the pair is both optional and dependent and the client did not deny it. */
export function judgePair(pair: PairTally): PairJudgement | undefined {
  const counts: Partial<Record<ObservationKind, number>> = {};
  for (const kind of EXAMPLE_ORDER) if (count(pair, kind) > 0) counts[kind] = count(pair, kind);

  const flagged = count(pair, "flagged");
  if (count(pair, "notFlagged") > flagged) return undefined;
  const free = count(pair, "free");
  const dependentKinds = DEPENDENT_KINDS.filter((kind) => kind !== "serverDrop" || count(pair, kind) >= MIN_SERVER_DROP_CHARACTERS);
  const dependent = charactersOf(pair, dependentKinds);
  if ((free === 0 && flagged === 0) || dependent.size === 0) return undefined;
  const shown = charactersOf(pair, ["showB", "showT"]).size;
  if (shown > MAX_SHOWN) return undefined;

  // Chains rarely leak even one free character, so one already says a lot; the client flag settles it.
  const optional = Math.max(free > 0 ? 1 - 0.2 * 0.4 ** (free - 1) : 0, flagged > 0 ? 0.9 : 0);
  let doubt = 1;
  for (const kind of dependentKinds) doubt *= (1 - (DEPENDENT_WEIGHT[kind] ?? 0)) ** Math.min(count(pair, kind), 2);
  // A character that only got T after completing B (a chain) outweighs two that got it freely.
  const unlock = count(pair, "unlock");
  const notChain = free + unlock === 0 ? 1 : free / (free + 2 * unlock);
  const score = optional * (1 - doubt) * notChain * (shown > 0 ? SHOWN_PENALTY : 1);

  const evidence: Evidence[] = [];
  for (const kind of EXAMPLE_ORDER) {
    const example = pair.examples.get(kind)?.[0];
    if (example && evidence.length < 5) evidence.push({ ...example, text: `${kind}: ${example.text}` });
  }
  return {
    breadcrumb: pair.breadcrumb,
    target: pair.target,
    score: Math.round(score * 1000) / 1000,
    support: charactersOf(pair, [...OPTIONAL_KINDS, ...dependentKinds]).size,
    contradict: charactersOf(pair, CONTRA_KINDS).size,
    counts,
    evidence,
  };
}

function questName(catalog: QuestCatalog, questId: number): string {
  const quest: CatalogQuest | undefined = catalog.quests[questId];
  return quest?.name ? `${questId} "${quest.name}"` : String(questId);
}

/**
 * Both Questie fields for each judged pair, keeping one target per breadcrumb. T's follow-ups at the
 * same giver inherit "B hidden once done" and often tie with T; more characters reach T than its
 * follow-ups, so support breaks the tie toward the start of the chain.
 */
export function toCandidates(judgements: readonly PairJudgement[], catalog: QuestCatalog): RelationCandidate[] {
  const byBreadcrumb = new Map<number, PairJudgement[]>();
  for (const judgement of judgements) byBreadcrumb.set(judgement.breadcrumb, [...(byBreadcrumb.get(judgement.breadcrumb) ?? []), judgement]);

  const candidates: RelationCandidate[] = [];
  for (const options of byBreadcrumb.values()) {
    options.sort((a, b) => b.score - a.score || b.support - a.support);
    const [winner, ...others] = options;
    const counts = Object.entries(winner.counts)
      .map(([kind, n]) => `${kind} ${n}`)
      .join(", ");
    const alternatives = others.length > 0 ? `; other targets ${others.map((o) => `${o.target} (${o.score})`).join(", ")}` : "";
    const note = `${questName(catalog, winner.breadcrumb)} -> ${questName(catalog, winner.target)}; characters: ${counts}${alternatives}`;
    const shared = { score: winner.score, support: winner.support, contradict: winner.contradict, evidence: winner.evidence, note };
    candidates.push({ questId: winner.breadcrumb, field: "breadcrumbForQuestId", target: winner.target, ...shared });
    candidates.push({ questId: winner.target, field: "breadcrumbs", target: winner.breadcrumb, ...shared });
  }
  return candidates.sort((a, b) => b.score - a.score || a.questId - b.questId);
}

/** Quests on either side of a pair that enough characters had something to say about. */
export function coveredQuests(tally: Tally, minCharacters: number): number[] {
  const covered = new Set<number>();
  for (const pair of tally.pairs.values()) {
    if (charactersOf(pair, EXAMPLE_ORDER).size < minCharacters) continue;
    covered.add(pair.breadcrumb);
    covered.add(pair.target);
  }
  return [...covered].sort((a, b) => a - b);
}

export interface BreadcrumbResult {
  tally: Tally;
  judgements: PairJudgement[];
  candidates: RelationCandidate[];
  coveredQuestIds: number[];
}

/** Characters needed on a pair before its quests count as covered (for recall). */
export const MIN_COVERAGE_CHARACTERS = 2;

export function judgeTally(tally: Tally, catalog: QuestCatalog): BreadcrumbResult {
  const judgements = [...tally.pairs.values()].map(judgePair).filter((judgement): judgement is PairJudgement => judgement !== undefined);
  return { tally, judgements, candidates: toCandidates(judgements, catalog), coveredQuestIds: coveredQuests(tally, MIN_COVERAGE_CHARACTERS) };
}

/** In-memory version of the whole signal, for tests and small inputs. */
export function inferBreadcrumbs(episodes: readonly QuestEpisode[], catalog: QuestCatalog, params: BreadcrumbParams = DEFAULT_PARAMS): BreadcrumbResult {
  const collector = new LinkCollector(catalog, params);
  for (const episode of episodes) collector.add(episode);
  const links = collector.finish();
  const tally = new Tally();
  for (const episode of episodes) tally.add(episode.characterKey, observeEpisode(episode, links, catalog, params));
  return judgeTally(tally, catalog);
}
