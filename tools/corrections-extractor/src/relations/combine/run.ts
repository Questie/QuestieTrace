// `npm run relations -- combine [--min-score n]`: all candidates -> reviewed relations + Questie output.
//
// ./inputs.ts loads every candidates/*.json and sets Wowhead aside as context only; ./pipeline.ts
// scores and resolves the rest. This file writes the outputs: output/relations.json (every quest
// with accepted or near-miss relations), output/foreverQuestRelationTraces.lua (Forever-new quests
// only), reports/combine-review.md and reports/combine-metrics.json.

import { mkdirSync, writeFileSync } from "fs";
import { resolve } from "path";
import { writeJsonAtomic } from "../core/io";
import { paths } from "../core/paths";
import type { RelationField, RelationSet } from "../core/types";
import { DEFAULT_MODEL_PARAMS, type ScoredEdge } from "./crossval";
import { domainOf, type Claim, type Edge, type QuestDomains } from "./edges";
import { loadInputs } from "./inputs";
import { foreverRecords, foreverRelationsLua } from "./lua";
import { formatComparison, formatMetrics, heldOutMetrics, METRIC_CAVEATS } from "./metrics";
import { combine, DEFAULTS, type Combined } from "./pipeline";
import type { Resolution } from "./resolve";
import { renderReview } from "./review";
import { buildQuestViews, type QuestView } from "./views";

const outputDir = resolve(paths.relationsDir, "output");
const relationsPath = resolve(outputDir, "relations.json");
const luaPath = resolve(outputDir, "foreverQuestRelationTraces.lua");
const reviewPath = resolve(paths.reportsDir, "combine-review.md");
const metricsPath = resolve(paths.reportsDir, "combine-metrics.json");

function parseMinScore(args: string[]): number {
  const index = args.indexOf("--min-score");
  if (index < 0) return DEFAULTS.minScore;
  const value = Number(args[index + 1]);
  if (!Number.isFinite(value) || value <= 0 || value > 1) throw new Error(`--min-score needs a number in (0, 1], got ${args[index + 1]}`);
  return value;
}

/**
 * Classic edges the model leaves out although at least two signals claim them at 0.8 or more, and
 * the inherited data lacks them. The model's Classic weights come from that inherited data, so a
 * chain Forever re-wired is distrusted precisely because the reference is stale; a person decides.
 */
function classicMisses(scored: readonly ScoredEdge[], decisions: Resolution["decisions"], truth: ReadonlyMap<number, RelationSet>, domains: QuestDomains) {
  const INHERITED_FIELDS: Partial<Record<Edge["kind"], RelationField[]>> = {
    prerequisite: ["preQuestSingle", "preQuestGroup"],
    next: ["nextQuestInChain"],
    breadcrumb: ["breadcrumbForQuestId"],
    exclusive: ["exclusiveTo"],
  };
  return scored
    .filter((edge) => domainOf(edge, domains) === "classic" && edge.label === false && decisions.get(edge.key)?.outcome === "below-threshold")
    .filter((edge) => new Set(edge.claims.filter((claim) => claim.score >= 0.8).map((claim) => claim.signal)).size >= 2)
    .sort((a, b) => b.probability - a.probability)
    .map((edge) => ({
      edge,
      inherited: (INHERITED_FIELDS[edge.kind] ?? []).flatMap((field) => (truth.get(edge.quest)?.[field] ?? []).map((target) => ({ field, target }))),
    }));
}

const claimJson = (claim: Claim) => ({ source: claim.source, score: claim.score, support: claim.support, contradict: claim.contradict });

function questJson(view: QuestView) {
  return {
    name: view.name,
    domain: view.domain,
    reference: view.reference,
    relations: view.relations,
    edges: view.edges.map((edge) => ({
      field: edge.field,
      target: edge.target,
      probability: Number(edge.probability.toFixed(4)),
      heldOut: edge.heldOut,
      outcome: edge.outcome,
      because: edge.because,
      status: edge.status,
      signals: edge.claims.map(claimJson),
      context: edge.context.length > 0 ? edge.context.map(claimJson) : undefined,
      logOdds: Object.fromEntries(edge.contributions.map((term) => [term.signal, Number(term.logOdds.toFixed(3))])),
    })),
    referenceOnly: view.referenceOnly.length > 0 ? view.referenceOnly : undefined,
    hints: view.hints.length > 0 ? view.hints.map((hint) => ({ field: hint.field, target: hint.target, context: hint.claims.map(claimJson) })) : undefined,
    notes: view.notes.length > 0 ? view.notes : undefined,
  };
}

export async function run(args: string[]): Promise<void> {
  const minScore = parseMinScore(args);
  const generatedAt = new Date();
  const inputs = loadInputs();
  // The same pipeline twice: once as shipped, once without signal-score columns (see METRIC_CAVEATS).
  const shipped = combine(inputs, minScore);
  const { scored, fitted, accepted, andSources } = shipped;
  // Single-signal comparisons use inputs.files only: Wowhead's agreement with the ground truth is circular.
  const metricsOf = (run: Combined) => heldOutMetrics(run.accepted.relations, run.atReviewScore.relations, inputs.files, inputs.groundTruth);
  const metrics = metricsOf(shipped);
  const labelFree = metricsOf(combine(inputs, minScore, { ...DEFAULT_MODEL_PARAMS, scoreFeatures: false }));
  writeJsonAtomic(metricsPath, {
    generatedAt: generatedAt.toISOString(),
    minScore,
    reviewScore: DEFAULTS.reviewScore,
    caveats: METRIC_CAVEATS,
    andSources,
    metrics,
    labelFree,
  });

  const views = buildQuestViews(scored, accepted, inputs.context, inputs.truth, inputs.domains, inputs.catalog.quests, DEFAULTS.reviewScore);
  const inputSummary = inputs.files.map((file) => ({
    signal: file.signal,
    generatedAt: file.generatedAt,
    inputCount: file.inputCount,
    candidates: file.candidates.length,
    covered: file.coveredQuestIds.length,
  }));
  writeJsonAtomic(relationsPath, {
    generatedAt: generatedAt.toISOString(),
    params: { ...DEFAULTS, minScore, ...DEFAULT_MODEL_PARAMS },
    inputs: inputSummary,
    contextOnly: inputs.contextFiles.map((file) => file.signal),
    andSources,
    models: fitted.map((kind) => ({
      kind: kind.kind,
      judged: kind.judged,
      positives: kind.positives,
      calibrated: Number.isFinite(kind.model.intercept),
      intercept: Number.isFinite(kind.model.intercept) ? kind.model.intercept : null,
      weights: Object.fromEntries(kind.columns.map((column, j) => [column.name, Number(kind.model.weights[j].toFixed(4))])),
    })),
    quests: Object.fromEntries([...views.values()].filter((view) => view.edges.length > 0).map((view) => [view.questId, questJson(view)])),
  });

  const authored = inputs.groundTruth.tiers.authoredForever;
  const records = foreverRecords(accepted.relations, authored, inputs.domains.isForeverNew);
  const luaQuests = records.size;
  mkdirSync(outputDir, { recursive: true });
  writeFileSync(
    luaPath,
    foreverRelationsLua(records, {
      candidateFiles: inputs.files.map((file) => file.signal),
      sessionCount: Math.max(...inputs.files.map((file) => file.inputCount)),
      minScore,
      generatedAt,
    }),
  );

  const metricsMarkdown = formatMetrics(metrics, minScore, DEFAULTS.reviewScore);
  mkdirSync(paths.reportsDir, { recursive: true });
  writeFileSync(
    reviewPath,
    renderReview({
      views,
      quests: inputs.catalog.quests,
      minScore,
      reviewScore: DEFAULTS.reviewScore,
      candidateFiles: inputSummary,
      contextFiles: inputs.contextFiles.map((file) => ({ signal: file.signal, candidates: file.candidates.length })),
      andSources,
      metricsMarkdown,
      labelFreeMarkdown: formatComparison(metrics, labelFree, minScore),
      caveats: METRIC_CAVEATS,
      fitted,
      luaPath,
      luaQuests,
      classicMisses: classicMisses(scored, accepted.decisions, inputs.truth, inputs.domains),
      generatedAt,
    }),
  );

  printSummary([...views.values()], accepted.relations, luaQuests, metricsMarkdown, inputs.files.length);
}

function printSummary(views: QuestView[], relations: ReadonlyMap<number, RelationSet>, luaQuests: number, metricsMarkdown: string, fileCount: number): void {
  const written = (view: QuestView) => view.edges.filter((edge) => ["accepted", "clique", "derived"].includes(edge.outcome));
  const forever = views.filter((view) => view.domain === "forever");
  const uncovered = forever.filter((view) => view.reference === "none" && written(view).length > 0);
  const authored = forever.filter((view) => view.reference === "authored");
  const statusCount = (list: QuestView[], status: string) => list.reduce((sum, view) => sum + written(view).filter((edge) => edge.status === status).length, 0);
  const classic = views.filter((view) => view.domain === "classic" && written(view).some((edge) => edge.status !== "agree"));
  console.log(
    `combine: ${fileCount} candidate files, ${relations.size} quests with accepted relations\n` +
      `  Forever-new without authored relations: ${uncovered.length} quests, ${uncovered.reduce((sum, view) => sum + written(view).length, 0)} relations\n` +
      `  Forever-new authored: agree ${statusCount(authored, "agree")}, new ${statusCount(authored, "new")}, conflict ${statusCount(authored, "conflict")}\n` +
      `  Classic quests disagreeing with inherited data: ${classic.length} (review only)\n` +
      `  -> ${luaPath} (${luaQuests} quests)\n  -> ${relationsPath}\n  -> ${reviewPath}\n\nHeld-out metrics:\n${metricsMarkdown}`,
  );
}
