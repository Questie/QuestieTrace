// Scores a signal's candidates against the ground truth (precision / recall per
// field and per tier). This is how signals are tuned: run the signal, run
// `npm run relations -- score <signal>`, adjust, repeat.
//
// Precision only counts candidates for quests the tier knows about.
// Recall only counts truth edges for quests the signal said it covered
// (CandidateFile.coveredQuestIds), so a signal is not punished for quests it
// never saw. "prerequisiteAny" merges preQuestSingle and preQuestGroup on both
// sides, since telling OR from AND is harder than finding the prerequisite.

import type { CandidateFile, GroundTruth, RelationField, RelationSet } from "./types";
import { RELATION_FIELDS } from "./types";

export type TierName = keyof GroundTruth["tiers"];

export interface FieldScore {
  field: RelationField | "prerequisiteAny";
  /** Whether the signal produced any candidate for this field at all. */
  emitted: boolean;
  truePositives: number;
  falsePositives: number;
  falseNegatives: number;
  precision: number;
  recall: number;
}

export interface TierScore {
  tier: TierName;
  /** Candidates whose quest is in this tier. */
  judgedCandidates: number;
  fields: FieldScore[];
  /** A few false positives, for review. */
  falsePositiveExamples: string[];
}

export interface ScoreReport {
  signal: string;
  minScore: number;
  tiers: TierScore[];
}

function valuesOf(set: RelationSet | undefined, field: RelationField | "prerequisiteAny"): Set<number> {
  if (!set) return new Set();
  if (field === "prerequisiteAny") return new Set([...(set.preQuestSingle ?? []), ...(set.preQuestGroup ?? [])]);
  return new Set(set[field] ?? []);
}

function ratio(numerator: number, denominator: number): number {
  return denominator === 0 ? 0 : numerator / denominator;
}

export function scoreCandidates(file: CandidateFile, truth: GroundTruth, options: { minScore?: number } = {}): ScoreReport {
  const minScore = options.minScore ?? 0;
  const accepted = file.candidates.filter((candidate) => candidate.score >= minScore);
  const covered = new Set(file.coveredQuestIds);
  const fields: Array<RelationField | "prerequisiteAny"> = [...RELATION_FIELDS, "prerequisiteAny"];

  const tiers: TierScore[] = [];
  for (const tierName of Object.keys(truth.tiers) as TierName[]) {
    const tier = truth.tiers[tierName];
    const inTier = accepted.filter((candidate) => tier[candidate.questId] !== undefined);
    const falsePositiveExamples: string[] = [];

    const fieldScores = fields.map((field): FieldScore => {
      const isPrerequisiteAny = field === "prerequisiteAny";
      const matches = (candidateField: RelationField) =>
        isPrerequisiteAny ? candidateField === "preQuestSingle" || candidateField === "preQuestGroup" : candidateField === field;

      // Deduplicate edges: several candidates may claim the same (quest, target) for prerequisiteAny.
      const claimed = new Map<string, { questId: number; target: number }>();
      for (const candidate of inTier) {
        if (matches(candidate.field)) claimed.set(`${candidate.questId}:${candidate.target}`, candidate);
      }

      let truePositives = 0;
      let falsePositives = 0;
      for (const { questId, target } of claimed.values()) {
        if (valuesOf(tier[questId], field).has(target)) {
          truePositives++;
        } else {
          falsePositives++;
          if (falsePositiveExamples.length < 15) falsePositiveExamples.push(`${field}: ${questId} -> ${target}`);
        }
      }

      // Recall only looks at covered quests, on both sides of the ratio.
      let coveredHits = 0;
      let falseNegatives = 0;
      for (const questId of covered) {
        for (const target of valuesOf(tier[questId], field)) {
          if (claimed.has(`${questId}:${target}`)) coveredHits++;
          else falseNegatives++;
        }
      }

      return {
        field,
        emitted: file.candidates.some((candidate) => matches(candidate.field)),
        truePositives,
        falsePositives,
        falseNegatives,
        precision: ratio(truePositives, truePositives + falsePositives),
        recall: ratio(coveredHits, coveredHits + falseNegatives),
      };
    });

    tiers.push({ tier: tierName, judgedCandidates: inTier.length, fields: fieldScores, falsePositiveExamples });
  }

  return { signal: file.signal, minScore, tiers };
}

/** Plain-text table of the fields a signal emitted, one block per tier. */
export function formatScoreReport(report: ScoreReport): string {
  const lines = [`signal ${report.signal} (minScore ${report.minScore})`];
  for (const tier of report.tiers) {
    lines.push(`  ${tier.tier}: ${tier.judgedCandidates} judged candidates`);
    lines.push("    field                     TP     FP     FN   precision  recall");
    for (const field of tier.fields.filter((score) => score.emitted)) {
      lines.push(
        `    ${field.field.padEnd(22)} ${String(field.truePositives).padStart(5)} ${String(field.falsePositives).padStart(6)} ` +
          `${String(field.falseNegatives).padStart(6)}   ${field.precision.toFixed(3).padStart(8)} ${field.recall.toFixed(3).padStart(7)}`,
      );
    }
    if (tier.falsePositiveExamples.length > 0) lines.push(`    false positives e.g. ${tier.falsePositiveExamples.slice(0, 5).join("; ")}`);
  }
  return lines.join("\n");
}
