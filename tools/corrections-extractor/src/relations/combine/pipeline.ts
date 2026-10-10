// The combine pipeline in memory, without reading or writing files: canonical edges from the
// trusted candidate files (./edges.ts), out-of-fold scores (./crossval.ts), then one consistent
// RelationSet per quest (./resolve.ts) with the ground-truth-dependent rule inputs measured
// fold-wise (./foldwise.ts). run.ts writes the outputs; the tests run this directly.
//
// Only CombineInputs.files feed it. Context-only files (Wowhead) never reach this module.

import { cvGroups, DEFAULT_MODEL_PARAMS, scoreEdges, type FittedKind, type ModelParams, type ScoredEdge } from "./crossval";
import { collectEdges, type Edge } from "./edges";
import { andSourcesByQuest } from "./foldwise";
import type { CombineInputs } from "./inputs";
import { resolveRelations, type Resolution, type ResolveOptions } from "./resolve";

export const DEFAULTS = {
  /** Accepted relations: relations.json and the Lua output. */
  minScore: 0.9,
  /** Near misses shown in the review, and the second metrics column. */
  reviewScore: 0.5,
  cliqueMinScore: 0.5,
};

export interface Combined {
  edges: Edge[];
  scored: ScoredEdge[];
  fitted: FittedKind[];
  /** Resolved at the acceptance threshold: what relations.json and the Lua output contain. */
  accepted: Resolution;
  /** Resolved at the review threshold, for the second metrics column. */
  atReviewScore: Resolution;
  /** Sources trusted for AND evidence on quests without ground truth. */
  andSources: string[];
}

export function combine(inputs: CombineInputs, minScore: number = DEFAULTS.minScore, params: ModelParams = DEFAULT_MODEL_PARAMS): Combined {
  const edges = [...collectEdges(inputs.files).values()];
  const groupOfQuest = cvGroups(edges, inputs.familyOf);
  const andSources = andSourcesByQuest(edges, { truth: inputs.truth, groupOfQuest, folds: params.folds });
  const { scored, fitted } = scoreEdges(edges, inputs.truth, inputs.coverage, groupOfQuest, params);
  const options = (threshold: number): ResolveOptions => ({
    minScore: threshold,
    knownQuest: (questId) => inputs.catalog.quests[String(questId)] !== undefined,
    cliqueMinScore: DEFAULTS.cliqueMinScore,
    familyOf: inputs.familyOf,
    andSourcesFor: andSources.forQuest,
  });
  return {
    edges,
    scored,
    fitted,
    accepted: resolveRelations(scored, options(minScore)),
    atReviewScore: resolveRelations(scored, options(DEFAULTS.reviewScore)),
    andSources: [...andSources.all],
  };
}
