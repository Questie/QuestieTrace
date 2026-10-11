// Canonical relation edges. Signals describe one relationship through different fields and
// directions (breadcrumbs on the target vs breadcrumbForQuestId on the breadcrumb, exclusiveTo on
// either side, preQuestSingle vs preQuestGroup), so every candidate becomes a claim on one
// canonical edge and all evidence about the same relationship lines up in one place.

import { isForeverQuestId } from "../core/eligibility";
import type { CandidateFile, Evidence, GroundTruth, RelationField, RelationSet } from "../core/types";

/**
 * - prerequisite: `quest` needs `target` (preQuestSingle or preQuestGroup; OR vs AND is decided per
 *   quest during resolution).
 * - next: `quest`.nextQuestInChain = `target`.
 * - breadcrumb: `quest` is a breadcrumb leading to `target` (breadcrumbForQuestId on `quest`,
 *   breadcrumbs on `target`).
 * - exclusive: unordered pair, stored with `quest` < `target`; emitted on both sides.
 * - the rest: `quest`.<kind> = `target`, as the field of the same name.
 */
export type EdgeKind =
  "prerequisite" | "next" | "breadcrumb" | "exclusive" | "parentQuest" | "availableUntilCompleted" | "availableStartingWith" | "disabledByQuest";

export const EDGE_KINDS: readonly EdgeKind[] = [
  "prerequisite",
  "next",
  "breadcrumb",
  "exclusive",
  "parentQuest",
  "availableUntilCompleted",
  "availableStartingWith",
  "disabledByQuest",
];

/** One signal's statement about an edge, in the field it used. */
export interface Claim {
  /** `<signal>/<field>`, with mirrored fields folded (breadcrumbs -> breadcrumbForQuestId). Calibrated as one unit. */
  source: string;
  signal: string;
  field: RelationField;
  score: number;
  support: number;
  contradict: number;
  evidence: Evidence[];
  note?: string;
}

export interface Edge {
  key: string;
  kind: EdgeKind;
  quest: number;
  target: number;
  claims: Claim[];
}

export type Domain = "forever" | "classic";

export function edgeKey(kind: EdgeKind, quest: number, target: number): string {
  return `${kind}:${quest}:${target}`;
}

/** Maps a candidate's (questId, field, target) onto its canonical edge, plus the field its source is filed under. */
export function canonicalize(
  questId: number,
  field: RelationField,
  target: number,
): { kind: EdgeKind; quest: number; target: number; sourceField: RelationField } {
  switch (field) {
    case "preQuestSingle":
    case "preQuestGroup":
      return { kind: "prerequisite", quest: questId, target, sourceField: field };
    case "nextQuestInChain":
      return { kind: "next", quest: questId, target, sourceField: field };
    case "breadcrumbForQuestId":
      return { kind: "breadcrumb", quest: questId, target, sourceField: "breadcrumbForQuestId" };
    case "breadcrumbs":
      return { kind: "breadcrumb", quest: target, target: questId, sourceField: "breadcrumbForQuestId" };
    case "exclusiveTo":
      return { kind: "exclusive", quest: Math.min(questId, target), target: Math.max(questId, target), sourceField: field };
    default:
      return { kind: field, quest: questId, target, sourceField: field };
  }
}

/**
 * Folds every candidate into canonical edges. When one source claims the same edge twice (both
 * sides of a breadcrumb or an exclusive pair), the stronger claim is kept: the second is the same
 * observation written on the other quest, not new evidence.
 */
export function collectEdges(files: readonly CandidateFile[]): Map<string, Edge> {
  const edges = new Map<string, Edge>();
  for (const file of files) {
    for (const candidate of file.candidates) {
      if (candidate.questId === candidate.target) continue;
      const { kind, quest, target, sourceField } = canonicalize(candidate.questId, candidate.field, candidate.target);
      const key = edgeKey(kind, quest, target);
      let edge = edges.get(key);
      if (!edge) {
        edge = { key, kind, quest, target, claims: [] };
        edges.set(key, edge);
      }
      const claim: Claim = {
        source: `${file.signal}/${sourceField}`,
        signal: file.signal,
        field: candidate.field,
        score: candidate.score,
        support: candidate.support,
        contradict: candidate.contradict,
        evidence: candidate.evidence,
        note: candidate.note,
      };
      const existing = edge.claims.findIndex((other) => other.source === claim.source);
      if (existing < 0) edge.claims.push(claim);
      else if (claim.score > edge.claims[existing].score) edge.claims[existing] = claim;
    }
  }
  return edges;
}

/** Quests whose relations an edge writes. Exclusive pairs and breadcrumbs write both sides. */
export function questsOf(edge: Pick<Edge, "kind" | "quest" | "target">): number[] {
  return edge.kind === "exclusive" || edge.kind === "breadcrumb" ? [edge.quest, edge.target] : [edge.quest];
}

/**
 * Which reference data judges a quest, and whether it is Forever-new. Ground-truth tier
 * membership decides, not the id alone: legacy Classic-Era quests can carry Forever-range ids
 * (65593-65610) and still be inherited data, review-only like any Classic quest.
 */
export interface QuestDomains {
  referenceOf(questId: number): "authored" | "inherited" | "none";
  isForeverNew(questId: number): boolean;
}

export function questDomains(truth: GroundTruth): QuestDomains {
  const authored = new Set(Object.keys(truth.tiers.authoredForever).map(Number));
  const inherited = new Set(Object.keys(truth.tiers.inheritedClassic).map(Number));
  return {
    referenceOf: (questId) => (authored.has(questId) ? "authored" : inherited.has(questId) ? "inherited" : "none"),
    isForeverNew: (questId) => !inherited.has(questId) && (authored.has(questId) || isForeverQuestId(questId)),
  };
}

/**
 * Edges touching a Forever-new quest are Forever edges, everything else Classic. Two-sided edges
 * count as Forever when either side is Forever-new, since the Forever side is what the Lua output
 * would change.
 */
export function domainOf(edge: Pick<Edge, "kind" | "quest" | "target">, domains: Pick<QuestDomains, "isForeverNew">): Domain {
  return questsOf(edge).some((questId) => domains.isForeverNew(questId)) ? "forever" : "classic";
}

/** Flattened ground truth: one RelationSet per quest across both tiers (tiers never share a quest). */
export function truthLookup(truth: GroundTruth): Map<number, RelationSet> {
  const lookup = new Map<number, RelationSet>();
  for (const tier of Object.values(truth.tiers)) {
    for (const [questId, set] of Object.entries(tier)) lookup.set(Number(questId), set);
  }
  return lookup;
}

/** Whether the ground truth judges this edge at all: one of the quests it writes has reference data. */
export function isJudged(edge: Pick<Edge, "kind" | "quest" | "target">, truth: ReadonlyMap<number, RelationSet>): boolean {
  return questsOf(edge).some((questId) => truth.has(questId));
}

/** Whether the ground truth contains this edge, in either of the fields that can express it. */
export function truthHas(edge: Pick<Edge, "kind" | "quest" | "target">, truth: ReadonlyMap<number, RelationSet>): boolean {
  const has = (questId: number, field: RelationField, value: number) => truth.get(questId)?.[field]?.includes(value) ?? false;
  switch (edge.kind) {
    case "prerequisite":
      return has(edge.quest, "preQuestSingle", edge.target) || has(edge.quest, "preQuestGroup", edge.target);
    case "next":
      return has(edge.quest, "nextQuestInChain", edge.target);
    case "breadcrumb":
      return has(edge.quest, "breadcrumbForQuestId", edge.target) || has(edge.target, "breadcrumbs", edge.quest);
    case "exclusive":
      return has(edge.quest, "exclusiveTo", edge.target) || has(edge.target, "exclusiveTo", edge.quest);
    default:
      return has(edge.quest, edge.kind, edge.target);
  }
}
