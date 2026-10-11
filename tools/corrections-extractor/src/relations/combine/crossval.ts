// Fits one model per edge kind and scores every edge, keeping ground truth out of its own scores.
//
// Edges the ground truth judges are scored out of fold: K-fold cross-validation over quest groups
// (see cvGroups), so this model never saw the label of an edge it scores. Edges nothing judges
// (Forever-new quests without authored relations) are scored by the model fitted on all ground
// truth. The same numbers drive both the held-out metrics and the output.
//
// Held out for this model only: the signals calibrated their own score tables on the same ground
// truth, so a claim's score already carries some of its label. Re-fitting every signal per fold
// is out of scope; instead ModelParams.scoreFeatures = false fits on the label-free columns alone
// (claimed, support, contradiction rate, silence) as a sanity check on how optimistic that is.
//
// One model per kind covers both domains. Separate Forever and Classic models (shrunk toward the
// pooled one) and a domain column were both tried and scored worse held-out log-loss on almost
// every kind; the signals' own scores already carry most domain differences. (Measured 2026-10
// while Wowhead was still a model input, not re-checked since.)

import { EDGE_KINDS, isJudged, truthHas, type Edge, type EdgeKind } from "./edges";
import { buildFeatureSpace, type Coverage, type FeatureColumn } from "./features";
import { fitLogistic, predict, type LogisticModel } from "./logistic";
import type { RelationSet } from "../core/types";

export interface ModelParams {
  folds: number;
  ridge: number;
  /** False drops every signal-score column (see the header). */
  scoreFeatures: boolean;
}

export const DEFAULT_MODEL_PARAMS: ModelParams = { folds: 5, ridge: 1, scoreFeatures: true };

export interface Contribution {
  /** A signal name, or "base" for the intercept. */
  signal: string;
  /** Log-odds this signal's columns added (claims positive or negative, silence usually negative). */
  logOdds: number;
}

export interface ScoredEdge extends Edge {
  probability: number;
  /** Ground-truth label, for edges the ground truth judges. */
  label?: boolean;
  /** True when scored out of fold (judged edges); false when scored by the model fitted on all ground truth. */
  heldOut: boolean;
  contributions: Contribution[];
}

export interface FittedKind {
  kind: EdgeKind;
  columns: FeatureColumn[];
  model: LogisticModel;
  judged: number;
  positives: number;
}

/** FNV-1a: stable across runs and machines, unlike anything seeded. */
function hash(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h;
}

export function foldOf(group: string, folds: number): number {
  return hash(group) % folds;
}

/**
 * CV groups of quests, and through their `quest` side of edges. A group starts as a quest family
 * (same-name copies, since signals copy evidence between copies) and is joined with the other
 * family of every exclusive pair and breadcrumb: those relate two quests at once, and resolution
 * weighs them against the prerequisites, chains and exclusivity between the same two families, so
 * all of those must be held out together. Prerequisites and chains do not join groups: they link
 * nearly everything (4,941 of 7,667 judged edges in one group on the 2026-10 data, against 179
 * for the largest group this way).
 */
export function cvGroups(edges: readonly Pick<Edge, "kind" | "quest" | "target">[], familyOf: (questId: number) => string): (questId: number) => string {
  const parent = new Map<string, string>();
  const find = (family: string): string => {
    const up = parent.get(family);
    if (up === undefined || up === family) return family;
    const root = find(up);
    parent.set(family, root);
    return root;
  };
  for (const edge of edges) {
    if (edge.kind !== "exclusive" && edge.kind !== "breadcrumb") continue;
    const [a, b] = [find(familyOf(edge.quest)), find(familyOf(edge.target))];
    if (a !== b) parent.set(a, b);
  }
  return (questId) => find(familyOf(questId));
}

/** Splits rows into K folds by group: each group's rows land in exactly one test fold. */
export function crossValidationSplits<T>(rows: readonly T[], groupOfRow: (row: T) => string, folds: number): Array<{ train: T[]; test: T[] }> {
  return Array.from({ length: folds }, (_, fold) => ({
    train: rows.filter((row) => foldOf(groupOfRow(row), folds) !== fold),
    test: rows.filter((row) => foldOf(groupOfRow(row), folds) === fold),
  }));
}

/** Below this much ground truth a kind cannot be calibrated: one with 3 judged edges, all true, would accept anything. */
const MIN_JUDGED = 10;
const MIN_PER_OUTCOME = 3;

interface Row {
  edge: Edge;
  x: number[];
  label?: number;
}

function explain(model: LogisticModel, columns: FeatureColumn[], x: number[]): Contribution[] {
  const bySignal = new Map<string, number>();
  x.forEach((value, j) => {
    if (value !== 0) bySignal.set(columns[j].signal, (bySignal.get(columns[j].signal) ?? 0) + model.weights[j] * value);
  });
  return [{ signal: "base", logOdds: model.intercept }, ...[...bySignal].map(([signal, logOdds]) => ({ signal, logOdds }))];
}

export function scoreEdges(
  edges: readonly Edge[],
  truth: ReadonlyMap<number, RelationSet>,
  coverage: Coverage,
  groupOfQuest: (questId: number) => string,
  params: ModelParams = DEFAULT_MODEL_PARAMS,
): { scored: ScoredEdge[]; fitted: FittedKind[] } {
  const scored: ScoredEdge[] = [];
  const fitted: FittedKind[] = [];
  for (const kind of EDGE_KINDS) {
    const ofKind = edges.filter((edge) => edge.kind === kind);
    if (ofKind.length === 0) continue;
    const space = buildFeatureSpace(kind, ofKind, coverage, { scores: params.scoreFeatures });
    const rows: Row[] = ofKind.map((edge) => ({
      edge,
      x: space.vector(edge),
      label: isJudged(edge, truth) ? Number(truthHas(edge, truth)) : undefined,
    }));
    const judged = rows.filter((row) => row.label !== undefined);
    // An explicit zero prior fixes the width even when a small kind leaves a training fold empty.
    const zero: LogisticModel = { intercept: 0, weights: space.columns.map(() => 0) };
    // Without enough ground truth an edge cannot be calibrated, so it scores 0 and is never accepted.
    const uncalibrated: LogisticModel = { intercept: -Infinity, weights: zero.weights };
    const positives = (train: Row[]) => train.filter((row) => row.label === 1).length;
    const fit = (train: Row[]) =>
      train.length < MIN_JUDGED || positives(train) < MIN_PER_OUTCOME || train.length - positives(train) < MIN_PER_OUTCOME
        ? uncalibrated
        : fitLogistic(
            train.map((row) => row.x),
            train.map((row) => row.label!),
            { ridge: params.ridge, prior: zero },
          );
    const emit = (row: Row, model: LogisticModel, heldOut: boolean) =>
      scored.push({
        ...row.edge,
        probability: predict(model, row.x),
        label: row.label === undefined ? undefined : row.label === 1,
        heldOut,
        contributions: explain(model, space.columns, row.x),
      });

    for (const { train, test } of crossValidationSplits(judged, (row) => groupOfQuest(row.edge.quest), params.folds)) {
      if (test.length === 0) continue;
      const model = fit(train);
      for (const row of test) emit(row, model, true);
    }
    const full = fit(judged);
    for (const row of rows) if (row.label === undefined) emit(row, full, false);
    fitted.push({ kind, columns: space.columns, model: full, judged: judged.length, positives: judged.filter((row) => row.label === 1).length });
  }
  return { scored, fitted };
}
