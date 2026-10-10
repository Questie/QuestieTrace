// Feature vectors for one edge kind's model. Each source (signal + field) that ever claims this
// kind gets a column for "claimed", plus its score and support as centered log terms when they
// vary, plus its contradiction rate when it reports any (a rate, not a count: busy chains collect
// contradictions too, and a raw count learned a positive weight). Each signal also gets a
// "silent" column: it covered the quest (CandidateFile.coveredQuestIds) and still did not claim
// the edge, which is the explicit counter-evidence the model can learn to penalize.

import { logit } from "./logistic";
import { questsOf, type Edge, type EdgeKind } from "./edges";

export type Coverage = ReadonlyMap<string, ReadonlySet<number>>;

export interface FeatureColumn {
  name: string;
  /** Signal the column belongs to, for grouping explanations. */
  signal: string;
}

export interface FeatureSpace {
  kind: EdgeKind;
  columns: FeatureColumn[];
  vector(edge: Edge): number[];
}

/** Scores of exactly 0 or 1 would make infinite log-odds. */
function scoreLogit(score: number): number {
  return logit(Math.min(0.995, Math.max(0.005, score)));
}

function mean(values: number[]): number {
  return values.reduce((sum, value) => sum + value, 0) / Math.max(1, values.length);
}

function varies(values: number[]): boolean {
  return values.some((value) => value !== values[0]);
}

/**
 * Built from every edge of the kind (labels are never looked at), so folds share one column layout.
 * `scores: false` leaves out the score columns: signals calibrated those on the ground truth.
 */
export function buildFeatureSpace(kind: EdgeKind, edges: readonly Edge[], coverage: Coverage, options: { scores: boolean } = { scores: true }): FeatureSpace {
  const claimsBySource = new Map<string, Edge["claims"]>();
  for (const edge of edges) {
    for (const claim of edge.claims) {
      const list = claimsBySource.get(claim.source) ?? [];
      list.push(claim);
      claimsBySource.set(claim.source, list);
    }
  }

  const columns: FeatureColumn[] = [];
  type Extract = (claim: Edge["claims"][number]) => number;
  const perSource: Array<{ source: string; extract: Extract[] }> = [];
  for (const source of [...claimsBySource.keys()].sort()) {
    const claims = claimsBySource.get(source)!;
    const signal = claims[0].signal;
    const extract: Extract[] = [() => 1];
    columns.push({ name: `${source} claimed`, signal });
    const logits = claims.map((claim) => scoreLogit(claim.score));
    if (options.scores && varies(logits)) {
      const center = mean(logits);
      extract.push((claim) => scoreLogit(claim.score) - center);
      columns.push({ name: `${source} score`, signal });
    }
    const supports = claims.map((claim) => Math.log1p(claim.support));
    if (varies(supports)) {
      const center = mean(supports);
      extract.push((claim) => Math.log1p(claim.support) - center);
      columns.push({ name: `${source} support`, signal });
    }
    if (claims.some((claim) => claim.contradict > 0)) {
      extract.push((claim) => claim.contradict / Math.max(1, claim.support + claim.contradict));
      columns.push({ name: `${source} contradict`, signal });
    }
    perSource.push({ source, extract });
  }

  const silentSignals = [...new Set([...claimsBySource.values()].map((claims) => claims[0].signal))].filter((signal) => coverage.has(signal)).sort();
  for (const signal of silentSignals) columns.push({ name: `${signal} silent`, signal });

  return {
    kind,
    columns,
    vector(edge: Edge): number[] {
      const x: number[] = [];
      for (const { source, extract } of perSource) {
        const claim = edge.claims.find((candidate) => candidate.source === source);
        for (const feature of extract) x.push(claim ? feature(claim) : 0);
      }
      for (const signal of silentSignals) {
        const covered = coverage.get(signal)!;
        const silent = !edge.claims.some((claim) => claim.signal === signal) && questsOf(edge).every((questId) => covered.has(questId));
        x.push(silent ? 1 : 0);
      }
      return x;
    },
  };
}
