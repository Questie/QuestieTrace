// Held-out precision/recall of the resolved relations per field and tier, next to the best any
// single signal manages. Scoring goes through core/score.ts so the numbers mean the same as
// `npm run relations -- score <signal>`.
//
// Both sides are scored over the same quests: the union of every signal's coveredQuestIds. A
// single signal is therefore also charged for quests only other signals saw, which is the fair
// question here: what would we get using that signal alone?

import { scoreCandidates, type FieldScore, type TierName } from "../core/score";
import type { CandidateFile, GroundTruth, RelationCandidate, RelationField, RelationSet } from "../core/types";

export const SINGLE_SIGNAL_MIN_SCORES = [0, 0.5, 0.8, 0.9, 0.95] as const;

/** How far "held out" goes. Written next to the metrics in combine-metrics.json and the review. */
export const METRIC_CAVEATS = [
  "Wowhead is context only. Questie's own relation data was scraped from Wowhead Series rows, so Wowhead's agreement with the ground " +
    "truth is circular. It adds no model column, coverage or silence, takes no part in resolution, and is left out of the " +
    "best-single-signal comparison.",
  "Held out for the combine model only. Every signal calibrated its own score table on groundtruth.json, so a claim's score already " +
    "carries some of its label and these metrics are optimistic. labelFree refits the model without any score column (claimed, support, " +
    "contradiction rate and silence only) as a sanity check; claimed is not fully label-free either, since signals drop candidates below " +
    "calibrated cut-offs.",
  "Cross-validated like the edge scores: which sources count as AND evidence.",
  "Chosen in-sample on all ground truth: the shape of the OR/AND rule (11 of 11 judged multi-prerequisite quests), suppressing " +
    "prerequisites between same-name copies of a breadcrumb and its target (106 of the 107 edges it drops are wrong), the model form " +
    "(one pooled model per kind over per-domain models, contradiction rate over count) and ridge 1.",
  "Fixed up front, not tuned: the 0.9 acceptance and 0.5 review thresholds, 0.5 for completing exclusive groups, 5 folds, and at " +
    "least 10 judged edges with 3 of each outcome before a kind is calibrated.",
];

export interface SingleSignalScore {
  signal: string;
  minScore: number;
  score: FieldScore;
}

export interface MetricsRow {
  field: RelationField | "prerequisiteAny";
  /** At the acceptance threshold (what the Lua output uses). */
  combined: FieldScore;
  /** At the lower review threshold. */
  review: FieldScore;
  /** Highest recall among single signals at no less than the combined precision, else the most precise one. */
  bestSingle?: SingleSignalScore;
}

export interface TierMetrics {
  tier: TierName;
  rows: MetricsRow[];
}

/** Writes resolved RelationSets as a CandidateFile so core/score.ts can judge them. */
export function relationsAsCandidates(name: string, relations: ReadonlyMap<number, RelationSet>, coveredQuestIds: number[]): CandidateFile {
  const candidates: RelationCandidate[] = [];
  for (const [questId, set] of relations) {
    for (const [field, targets] of Object.entries(set) as Array<[RelationField, number[]]>) {
      for (const target of targets) candidates.push({ questId, field, target, score: 1, support: 0, contradict: 0, evidence: [] });
    }
  }
  return { signal: name, generatedAt: new Date().toISOString(), inputCount: 0, params: {}, coveredQuestIds, candidates };
}

function pickBestSingle(combined: FieldScore, options: SingleSignalScore[]): SingleSignalScore | undefined {
  const useful = options.filter((option) => option.score.truePositives > 0);
  const atPrecision = useful.filter((option) => option.score.precision >= combined.precision - 0.005);
  if (atPrecision.length > 0) return atPrecision.reduce((a, b) => (b.score.recall > a.score.recall ? b : a));
  return useful.reduce<SingleSignalScore | undefined>(
    (a, b) => (!a || b.score.precision > a.score.precision || (b.score.precision === a.score.precision && b.score.recall > a.score.recall) ? b : a),
    undefined,
  );
}

export function heldOutMetrics(
  accepted: ReadonlyMap<number, RelationSet>,
  review: ReadonlyMap<number, RelationSet>,
  files: readonly CandidateFile[],
  truth: GroundTruth,
): TierMetrics[] {
  const universe = [...new Set(files.flatMap((file) => file.coveredQuestIds))].sort((a, b) => a - b);
  const combined = scoreCandidates(relationsAsCandidates("combine", accepted, universe), truth);
  const lower = scoreCandidates(relationsAsCandidates("combine-review", review, universe), truth);
  const singles = files.flatMap((file) =>
    SINGLE_SIGNAL_MIN_SCORES.map((minScore) => ({
      signal: file.signal,
      minScore,
      report: scoreCandidates({ ...file, coveredQuestIds: universe }, truth, { minScore }),
    })),
  );

  return combined.tiers.map((tier, tierIndex) => ({
    tier: tier.tier,
    rows: tier.fields
      .filter((field) => field.emitted || field.falseNegatives > 0)
      .map((field) => {
        const fieldIndex = tier.fields.indexOf(field);
        const options = singles
          .map(({ signal, minScore, report }) => ({ signal, minScore, score: report.tiers[tierIndex].fields[fieldIndex] }))
          .filter((option) => option.score.emitted);
        return { field: field.field, combined: field, review: lower.tiers[tierIndex].fields[fieldIndex], bestSingle: pickBestSingle(field, options) };
      }),
  }));
}

function cell(score: FieldScore): string {
  return `${score.precision.toFixed(3)} / ${score.recall.toFixed(3)} (${score.truePositives}/${score.falsePositives}/${score.falseNegatives})`;
}

/** Combined precision / recall at the acceptance threshold for two model variants, one row per field and tier. */
export function formatComparison(full: TierMetrics[], labelFree: TierMetrics[], minScore: number): string {
  const lines = [`| tier | field | full model ≥ ${minScore} P / R | without score columns ≥ ${minScore} P / R |`, "|---|---|---|---|"];
  full.forEach((tier, tierIndex) => {
    for (const row of tier.rows) {
      const other = labelFree[tierIndex].rows.find((candidate) => candidate.field === row.field);
      if (!row.combined.emitted && !other?.combined.emitted) continue;
      lines.push(`| ${tier.tier} | ${row.field} | ${cell(row.combined)} | ${other ? cell(other.combined) : "-"} |`);
    }
  });
  return lines.join("\n");
}

/** Markdown tables, one per tier. */
export function formatMetrics(metrics: TierMetrics[], minScore: number, reviewScore: number): string {
  const lines: string[] = [];
  for (const tier of metrics) {
    lines.push(`#### ${tier.tier}`, "");
    lines.push(`| field | combined ≥ ${minScore} P / R (TP/FP/FN) | combined ≥ ${reviewScore} P / R | best single signal | its P / R |`);
    lines.push("|---|---|---|---|---|");
    for (const row of tier.rows) {
      const best = row.bestSingle ? `${row.bestSingle.signal} ≥ ${row.bestSingle.minScore}` : "-";
      const bestScore = row.bestSingle ? cell(row.bestSingle.score) : "-";
      lines.push(`| ${row.field} | ${cell(row.combined)} | ${cell(row.review)} | ${best} | ${bestScore} |`);
    }
    lines.push("");
  }
  return lines.join("\n");
}
