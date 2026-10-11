// Provenance and base scores for Wowhead claims.
//
// Wowhead is untrusted: combine shows these claims to reviewers as context only and never lets
// them decide a relation. The scores order claims for that review, nothing more.
//
// A base score is the measured precision of that kind of claim against groundtruth.json, rounded
// down and shrunk for small samples. That measures agreement with Questie's current data (authored
// Forever fixes, inherited Classic relations), not with what Forever servers actually do, and the
// agreement is partly circular: Questie's existing relations were partly scraped from these same
// Series rows. Re-measure after changing the parser or mapping:
// `npm run relations -- source:wowhead`, then `npm run relations -- score wowhead-<surface>`; the
// candidate `note` carries field, shape and provenance for finer splits.

import type { RelationField } from "../../core/types";
import type { ClaimShape, Surface } from "./interpret";
import type { PageEnvironment } from "./parse";

/**
 * Where a claim comes from.
 * - forever-new-edge: on a page fetched from Wowhead's Forever environment, and the Era cache
 *   does not produce the same claim. The only Wowhead evidence about Forever changes.
 * - forever-era-edge: on a Forever page, and Era pages produce the same claim (inherited content).
 * - era-page: only on Era pages that scraper-questie copied into the Forever cache and never
 *   refetched. Era data, not Forever evidence.
 */
export type Provenance = "forever-new-edge" | "forever-era-edge" | "era-page";

export function provenanceOf(environment: PageEnvironment, key: string, eraClaimKeys: ReadonlySet<string>): Provenance {
  // An unknown environment cannot be shown to be Forever data, so it gets the inherited trust level.
  if (environment !== "forever") return "era-page";
  return eraClaimKeys.has(key) ? "forever-era-edge" : "forever-new-edge";
}

export function claimKey(claim: { surface: Surface; questId: number; field: RelationField; target: number }): string {
  return `${claim.surface}|${claim.questId}|${claim.field}|${claim.target}`;
}

/** Score for claim kinds that were never measured (no rows below). */
const UNMEASURED = 0.1;

// [surface, field, shape, scores by provenance]. Comments give TP/judged per provenance as
// measured on 2026-10-10 (forever-new-edge on authoredForever, the others on inheritedClassic):
// agreement with current Questie data, which can itself be wrong or behind Forever.
// Panels only exist on Era pages, so their other provenances are unmeasured guesses.
const SCORES: Array<[Surface, RelationField, ClaimShape, Record<Provenance, number>]> = [
  // 42/47, 863/875, 1/1
  ["series", "preQuestSingle", "single", { "forever-new-edge": 0.85, "forever-era-edge": 0.95, "era-page": 0.8 }],
  // 2/2, 88/98, 0/42 (Era pages list Era-only duplicates of a step)
  ["series", "preQuestSingle", "variants", { "forever-new-edge": 0.7, "forever-era-edge": 0.85, "era-page": 0.1 }],
  // 2/4, 16/17, none
  ["series", "preQuestSingle", "group", { "forever-new-edge": 0.5, "forever-era-edge": 0.8, "era-page": 0.4 }],
  // 6/14, 253/428, 0/1: the rest are breadcrumbs (below)
  ["series", "preQuestSingle", "lead-in", { "forever-new-edge": 0.4, "forever-era-edge": 0.55, "era-page": 0.3 }],
  // 39/48, 934/935, 1/1
  ["series", "nextQuestInChain", "single", { "forever-new-edge": 0.8, "forever-era-edge": 0.95, "era-page": 0.8 }],
  // 5/6, 418/425, 0/1
  ["series", "nextQuestInChain", "lead-in", { "forever-new-edge": 0.75, "forever-era-edge": 0.95, "era-page": 0.6 }],
  // 2/6, 161/425, 0/1
  ["series", "breadcrumbForQuestId", "lead-in", { "forever-new-edge": 0.25, "forever-era-edge": 0.35, "era-page": 0.2 }],
  // 2/10, 160/428, 0/1
  ["series", "breadcrumbs", "lead-in", { "forever-new-edge": 0.2, "forever-era-edge": 0.35, "era-page": 0.2 }],
  // unjudged: no Toxic Soil quest is in the ground truth yet
  ["storyline", "preQuestSingle", "single", { "forever-new-edge": 0.5, "forever-era-edge": 0.5, "era-page": 0.5 }],
  ["storyline", "preQuestSingle", "variants", { "forever-new-edge": 0.5, "forever-era-edge": 0.5, "era-page": 0.5 }],
  // era-page 97/101
  ["requires", "preQuestSingle", "single", { "forever-new-edge": 0.85, "forever-era-edge": 0.9, "era-page": 0.9 }],
  // era-page 8/8 (one page)
  ["requires", "preQuestSingle", "variants", { "forever-new-edge": 0.7, "forever-era-edge": 0.8, "era-page": 0.8 }],
  // never observed
  ["requires", "preQuestGroup", "group", { "forever-new-edge": 0.4, "forever-era-edge": 0.5, "era-page": 0.5 }],
  // era-page 52/329: repeatable turn-in groups that list themselves
  ["requires-any", "preQuestSingle", "with-self", { "forever-new-edge": 0.15, "forever-era-edge": 0.15, "era-page": 0.15 }],
  // era-page 3/3
  ["requires-in-progress", "parentQuest", "single", { "forever-new-edge": 0.7, "forever-era-edge": 0.75, "era-page": 0.75 }],
  // era-page 37/41
  ["unlocks", "preQuestSingle", "single", { "forever-new-edge": 0.8, "forever-era-edge": 0.85, "era-page": 0.85 }],
  // era-page 21/99
  ["unlocks", "preQuestSingle", "variants", { "forever-new-edge": 0.2, "forever-era-edge": 0.2, "era-page": 0.2 }],
  // era-page 0/147
  ["unlocks", "preQuestSingle", "with-self", { "forever-new-edge": 0.05, "forever-era-edge": 0.05, "era-page": 0.05 }],
  // era-page 36/229
  ["disables", "exclusiveTo", "variants", { "forever-new-edge": 0.15, "forever-era-edge": 0.15, "era-page": 0.15 }],
  // era-page 0/279
  ["disables", "exclusiveTo", "group", { "forever-new-edge": 0.05, "forever-era-edge": 0.05, "era-page": 0.05 }],
];

const SCORE_INDEX = new Map(SCORES.map(([surface, field, shape, byProvenance]) => [`${surface}|${field}|${shape}`, byProvenance]));

export function baseScore(surface: Surface, field: RelationField, shape: ClaimShape, provenance: Provenance): number {
  return SCORE_INDEX.get(`${surface}|${field}|${shape}`)?.[provenance] ?? UNMEASURED;
}
