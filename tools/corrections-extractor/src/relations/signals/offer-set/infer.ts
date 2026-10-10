// From per-quest assessments to prerequisite edges.
//
// Alive candidates include every quest characters finished before reaching X (starting zones,
// earlier hubs), so an edge needs evidence that ties P to X:
//
// - primary: P is turned in where X is given (link) AND P was the most recent completion before
//   characters first saw X (latest) AND, among characters who visited X's giver earlier, most
//   completed P only after that visit (window).
// - secondary: no primary exists for X, and P was the latest or an immediate completion with a
//   compatible window, seen by enough characters, none of whom first saw X right after levelling
//   to its required level (class-trainer quests unlock by level; what came just before is chance).
//   Mostly quests whose finisher neither the catalog nor the traces know.
//
// Then: drop P1 when another choice P2 implies it (P1 completed at every sighting of P2), and
// pick the field. One remaining choice is a preQuestSingle (with its faction-split OR members,
// co-completed copies and race-covering variants). Several independent choices, each the last
// completion for some characters, are a preQuestGroup. Play order alone cannot tell ALL-of from
// ANY-of when every character did all of them, so a group is "confirmed" only when, for each
// member, some character had it done at a complete list from X's giver that still did not offer
// X while another member was missing (see allOfConfirmed); unconfirmed groups score lower.

import { isForeverQuestId } from "../../core/eligibility";
import type { RelationCandidate } from "../../core/types";
import type { CandidateStats, QuestAssessment } from "./aggregate";
import type { VariantIndex } from "./variants";

export interface InferParams {
  /** Quests seen by fewer independent characters are not judged (and not covered). */
  minCharacters: number;
  /** Minimum share of window characters that completed P after reaching X's giver. */
  minPendingRate: number;
  /** Secondary edges need at least this many supporting characters. */
  minSecondarySupport: number;
}

export type EdgeTier = "primary" | "secondary";
export type PrerequisiteField = "preQuestSingle" | "preQuestGroup";

/** Features of one emitted edge; the score is a calibrated lookup over these. */
export interface EdgeFeatures {
  field: PrerequisiteField;
  /** The quest is Forever-new content (its truth is hand-authored, not inherited from Classic). */
  forever: boolean;
  tier: EdgeTier;
  /**
   * direct: P itself qualified; or: member of a faction-split group; copy: a same-name quest the
   * server completed together with P; variant: added to cover races no observed copy allows.
   */
  shape: "direct" | "or" | "copy" | "variant";
  characters: number;
  support: number;
  contradict: number;
  link: boolean;
  pending: number;
  background: number;
  fresh: number;
  immediate: number;
  latest: number;
  /** Share of characters that first saw X right after levelling to its required level. */
  levelUnlockedShare: number;
  /** Independent choices left for the quest after pruning (1 = preQuestSingle). */
  remaining: number;
  /** preQuestGroup only: every member was seen to be insufficient without another member. */
  allOfConfirmed: boolean;
}

export interface InferredEdge {
  questId: number;
  target: number;
  features: EdgeFeatures;
  stats: CandidateStats;
}

function windowAllows(stats: CandidateStats, params: InferParams): boolean {
  const window = stats.pending + stats.background;
  return window === 0 || stats.pending / window >= params.minPendingRate;
}

export function isPrimary(stats: CandidateStats, params: InferParams): boolean {
  return stats.link && stats.latest > 0 && windowAllows(stats, params);
}

export function isSecondary(stats: CandidateStats, params: InferParams): boolean {
  return stats.support >= params.minSecondarySupport && (stats.latest > 0 || stats.immediate > 0) && windowAllows(stats, params);
}

/**
 * For every member, some character had it completed at a complete list from X's giver (at a level
 * that allowed X) that did not offer X while another member was still missing. Under ANY-of that
 * list would have offered X, so each member was shown to be insufficient alone.
 */
function allOfConfirmed(members: number[], assessment: QuestAssessment): boolean {
  return members.every((member) =>
    assessment.blockedLists.some((missing) => !missing.includes(member) && members.some((other) => other !== member && missing.includes(other))),
  );
}

/** True when P1 was completed at every sighting of P2 (P2 implies P1) but not the other way round. */
function implied(p1: number, p2: number, assessments: ReadonlyMap<number, QuestAssessment>): boolean {
  const of2 = assessments.get(p2);
  if (!of2 || !of2.alive.some((entry) => entry.target === p1)) return false;
  const of1 = assessments.get(p1);
  return !of1?.alive.some((entry) => entry.target === p2);
}

/**
 * Same-name candidates completed at exactly the same moments as P for every character: some
 * Forever quests (e.g. "The Great Outdoors") flag all their copies completed on one turn-in, and
 * Questie lists every copy. Steps of a same-name chain complete at different times and stay out.
 *
 * Copies are kept even when their audience excludes X's (the Horde copy on an Alliance quest),
 * unlike variants: they are observed completed for these very characters, so they are harmless at
 * runtime, and Questie's authored Forever data lists them (164 of 170 such edges on the
 * authoredForever tier, 2026-10-10). On Classic quests Questie never lists Forever's copies,
 * which the score reflects.
 */
function coCompleted(stats: CandidateStats, assessment: QuestAssessment, variants: VariantIndex): CandidateStats[] {
  const name = variants.nameOf(stats.target);
  if (!name) return [];
  return assessment.alive.filter(
    (other) =>
      other.target !== stats.target &&
      variants.nameOf(other.target) === name &&
      other.support === stats.support &&
      other.fresh === stats.fresh &&
      other.immediate === stats.immediate &&
      other.latest === stats.latest &&
      other.pending === stats.pending,
  );
}

interface Choice {
  stats: CandidateStats;
  shape: "direct" | "or";
  /** Quests this choice stands for: the target, or every member of an OR group. */
  members: number[];
  /** Copies completed together with a direct target; they stand for the same requirement. */
  copies: CandidateStats[];
}

function choose(assessment: QuestAssessment, variants: VariantIndex, params: InferParams): { tier: EdgeTier; choices: Choice[] } {
  // Forever can flag another faction's same-name copy completed too; a copy nobody who can take
  // X could do is never X's own prerequisite (it can still ride along as a copy).
  const candidates = assessment.alive.filter((stats) => !variants.exclusiveAudiences(assessment.questId, stats.target));

  const build = (qualifies: (stats: CandidateStats) => boolean): Choice[] => {
    const choices: Choice[] = [];
    // Linked candidates first, so a co-completed group is represented by the copy given here.
    const ordered = candidates.filter(qualifies).sort((a, b) => Number(b.link) - Number(a.link) || a.target - b.target);
    for (const stats of ordered) {
      if (choices.some((choice) => choice.copies.some((copy) => copy.target === stats.target))) continue;
      choices.push({ stats, shape: "direct", members: [stats.target], copies: coCompleted(stats, assessment, variants) });
    }
    for (const group of assessment.orGroups) {
      if (qualifies(group.stats)) choices.push({ stats: group.stats, shape: "or", members: group.members, copies: [] });
    }
    return choices;
  };

  const primary = build((stats) => isPrimary(stats, params));
  if (primary.length > 0) return { tier: "primary", choices: primary };
  if (assessment.levelUnlocked > 0) return { tier: "secondary", choices: [] };
  return { tier: "secondary", choices: build((stats) => isSecondary(stats, params)) };
}

export function inferQuest(
  assessment: QuestAssessment,
  assessments: ReadonlyMap<number, QuestAssessment>,
  variants: VariantIndex,
  params: InferParams,
): InferredEdge[] {
  if (assessment.characters < params.minCharacters) return [];
  const questId = assessment.questId;
  const { tier, choices } = choose(assessment, variants, params);

  const impliedByOther = (choice: Choice) =>
    choices.some((other) => other !== choice && choice.members.every((p1) => other.members.some((p2) => p2 !== p1 && implied(p1, p2, assessments))));
  const kept = choices.filter((choice) => !impliedByOther(choice));
  const field: PrerequisiteField = kept.length >= 2 ? "preQuestGroup" : "preQuestSingle";
  const confirmed = field === "preQuestGroup" && allOfConfirmed(kept.map((choice) => choice.stats.target), assessment);

  const edges = new Map<number, InferredEdge>();
  const add = (target: number, stats: CandidateStats, shape: EdgeFeatures["shape"]) => {
    if (target === questId || edges.has(target)) return;
    edges.set(target, {
      questId,
      target,
      stats,
      features: {
        field,
        forever: isForeverQuestId(questId),
        tier,
        shape,
        characters: assessment.characters,
        support: stats.support,
        contradict: stats.contradict,
        link: stats.link,
        pending: stats.pending,
        background: stats.background,
        fresh: stats.fresh,
        immediate: stats.immediate,
        latest: stats.latest,
        levelUnlockedShare: assessment.levelUnlocked / assessment.characters,
        remaining: kept.length,
        allOfConfirmed: confirmed,
      },
    });
  };

  if (field === "preQuestGroup") {
    // A group entry is one quest; an OR of faction copies inside a group is not expressible.
    for (const choice of kept) if (choice.shape === "direct") add(choice.stats.target, choice.stats, "direct");
    return [...edges.values()];
  }

  for (const choice of kept) {
    for (const member of choice.members) {
      const own = assessment.alive.find((entry) => entry.target === member);
      add(member, choice.shape === "direct" ? choice.stats : (own ?? choice.stats), choice.shape);
    }
    for (const copy of choice.copies) add(copy.target, copy, "copy");
    // Races the chosen quests exclude would never see X; their same-name variants are what
    // characters of those races complete instead. A variant nobody in X's audience can take
    // (another race or class entirely) never helps.
    const covering = [...choice.members, ...choice.copies.map((copy) => copy.target)];
    if (variants.uncoveredRaces(questId, covering).length === 0) continue;
    for (const member of choice.members) {
      for (const variant of variants.variantsOf(member)) {
        if (!variants.exclusiveAudiences(questId, variant)) add(variant, { ...choice.stats, target: variant, evidence: [] }, "variant");
      }
    }
  }
  return [...edges.values()];
}

export function toCandidate(edge: InferredEdge, score: number): RelationCandidate {
  const f = edge.features;
  return {
    questId: edge.questId,
    field: f.field,
    target: edge.target,
    score,
    support: f.support,
    contradict: f.contradict,
    evidence: edge.stats.evidence,
    note:
      `${f.tier}/${f.shape}; ${f.support}/${f.characters} chars had it at every sighting; link=${f.link} latest=${f.latest} ` +
      `immediate=${f.immediate} fresh=${f.fresh} pending=${f.pending} background=${f.background} ` +
      `levelUnlocked=${f.levelUnlockedShare.toFixed(2)} remaining=${f.remaining}` +
      (f.field === "preQuestGroup" ? ` allOfConfirmed=${f.allOfConfirmed}` : ""),
  };
}
