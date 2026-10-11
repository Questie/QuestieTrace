// Pass 3 of the absence signal: which Questie field explains how a suspect blocked a quest.
//
// For suspect Y of blocked quest X (blocked.ts), the offer moments decide which states of Y
// are compatible with X being offered:
//
//   Y in log blocks | Y completed blocks | field
//   ----------------+--------------------+-------------------------------------------------------
//   yes             | yes                | exclusiveTo(X) ∋ Y, or nextQuestInChain(X) = Y when X
//                   |                    | hands off to Y (plus breadcrumbForQuestId when X is a
//                   |                    | lead-in from another giver)
//   yes             | no                 | breadcrumbs(X) ∋ Y when Y hands off to X, else
//                   |                    | disabledByQuest(X) = Y
//   no              | yes                | availableUntilCompleted(X) = Y
//
// "Yes" means no character was offered X in that state of Y; "no" means one was. A state never
// observed at all defaults to "yes", because exclusiveTo is by far the most common of these in
// Questie's data. exclusiveTo is one-directional: Y ∋ X needs blocked moments of Y of its own
// (inferring it from X's blocks alone was right 3 times out of 6).
//
// A missing prerequisite P becomes preQuestSingle(X) ∋ P, a missing same-name group one
// preQuestSingle edge per member, and a prerequisite that another prerequisite of X itself
// requires is folded into that one (Questie lists only the direct step).

import type { QuestCatalog, RelationCandidate, RelationField } from "../../core/types";
import { isForeverQuestId } from "../../core/eligibility";
import { attribute, type Share } from "./attribute";
import { explanationKey, type BlockedQuest, type BlockKind } from "./blocked";
import type { LocalReason, Locality } from "./locality";
import { bundleOf, pairKey, representative, type OfferIndex, type QuestOffers } from "./offers";
import type { AbsenceParams } from "./params";
import { scoreEdge, type ScoreGroup } from "./score";

interface Edge {
  questId: number;
  field: RelationField;
  target: number;
  /** For preQuestSingle edges, decided after ancestors are folded in (see scoreGroupOf). */
  group: ScoreGroup;
  shares: Map<string, Share>;
  /** Prerequisites only: characters blocked while the prerequisite was not even in their log. */
  untaken?: Set<string>;
  contradict: number;
  note: string;
}

type Explanations = Map<string, Map<string, Share>>;

export function classify(
  blockedQuests: Map<number, BlockedQuest>,
  index: OfferIndex,
  locality: Locality,
  catalog: QuestCatalog,
  params: AbsenceParams,
): RelationCandidate[] {
  const edges: Edge[] = [];
  for (const [questId, blocked] of blockedQuests) {
    const offers = index.quests.get(questId)!;
    const explanations = attribute(blocked, params);
    const targets = suspectsOf(explanations, index, catalog);
    const questEdges = [...targets].flatMap((target) => edgesFor(questId, target, explanations, offers, index, params));
    edges.push(...withoutAncestors(questEdges, index));
  }

  const candidates: RelationCandidate[] = [];
  for (const edge of edges) {
    const blocked = blockedQuests.get(edge.questId);
    const unexplainedRate = blocked && blocked.characters.size > 0 ? blocked.unexplained.size / blocked.characters.size : 0;
    const shares = [...edge.shares.values()].sort((a, b) => b.share - a.share);
    const local = bundleReason(locality, index, edge.questId, edge.target);
    const group = scoreGroupOf(edge);
    const score = Math.round(scoreEdge(group, shares.map((share) => share.share), local, isForeverQuestId(edge.questId)) * 1000) / 1000;
    if (score < params.minScore) continue;
    const likely = shares.filter((share) => share.share >= 0.5);
    candidates.push({
      questId: edge.questId,
      field: edge.field,
      target: edge.target,
      score,
      // Characters for whom this was the likely cause of their block; the note lists every share.
      support: likely.length,
      contradict: edge.contradict,
      evidence: shares.slice(0, 5).map((share) => share.evidence),
      note:
        `${edge.note}; group ${group}; local ${local ?? "none"}; transitions ${likely.filter((share) => share.transition).length}; ` +
        `shares ${formatShares(shares)}; unexplained ${(unexplainedRate * 100).toFixed(0)}%`,
    });
  }
  return candidates.sort((a, b) => a.questId - b.questId || a.field.localeCompare(b.field) || a.target - b.target);
}

/** A prerequisite proven by a character who had never taken it, or one only ever seen in the log. */
function scoreGroupOf(edge: Edge): ScoreGroup {
  if (edge.group !== "prerequisite" || !edge.untaken) return edge.group;
  const proven = [...edge.untaken].some((character) => (edge.shares.get(character)?.share ?? 0) >= 0.5);
  return proven ? "prerequisite" : "pendingPrerequisite";
}

function formatShares(shares: Share[]): string {
  const shown = shares.slice(0, 12).map((share) => share.share.toFixed(2));
  return shown.join(",") + (shares.length > shown.length ? `,...(${shares.length})` : "");
}

/** Copies that complete together are interchangeable, so a tie to any of them counts for all. */
function bundleReason(locality: Locality, index: OfferIndex, questId: number, target: number): LocalReason | undefined {
  const reasons = bundleOf(index, target).map((member) => locality.reason(questId, member));
  return reasons.find((reason) => reason === "sameName" || reason === "handoff") ?? reasons.find((reason) => reason !== undefined);
}

/**
 * Suspect quests, with completion bundles expanded: every copy gets its own edge, except hidden
 * flag quests (not in QuestieDB) that complete alongside a real one.
 */
function suspectsOf(explanations: Explanations, index: OfferIndex, catalog: QuestCatalog): Set<number> {
  const targets = new Set<number>();
  for (const key of explanations.keys()) {
    const questId = Number(key.slice(key.indexOf(":") + 1));
    const members = key.startsWith("inLog:") || key.startsWith("missingAny:") ? [questId] : bundleOf(index, questId);
    const known = members.filter((member) => catalog.quests[member] !== undefined);
    for (const member of known.length > 0 ? known : members) targets.add(member);
  }
  return targets;
}

function edgesFor(questId: number, target: number, explanations: Explanations, offers: QuestOffers, index: OfferIndex, params: AbsenceParams): Edge[] {
  const get = (kind: BlockKind) =>
    explanations.get(explanationKey(kind, kind === "inLog" || kind === "missingAny" ? target : representative(index, target)));
  const edges: Edge[] = [];

  const missing = get("missing");
  const pending = get("pending");
  if (missing || pending) {
    const untaken = new Set(missing?.keys() ?? []);
    const shares = bestOf(missing, pending);
    edges.push({ questId, field: "preQuestSingle", target, group: "prerequisite", shares, untaken, contradict: 0, note: "missing prerequisite" });
  }
  const missingAny = get("missingAny");
  if (missingAny) {
    const members = offers.requiredAny.get(target)!;
    for (const member of members) {
      const note = `missing any of ${members.join("/")}`;
      edges.push({ questId, field: "preQuestSingle", target: member, group: "anyPrerequisite", shares: missingAny, contradict: 0, note });
    }
  }

  const inLog = get("inLog");
  const completed = get("completed");
  if (!inLog && !completed) return edges;
  const logContradict = offers.inLogWhenOffered.get(target)?.size ?? 0;
  const completedContradict = offers.completedWhenOffered.get(target)?.size ?? 0;
  const logBlocks = logContradict <= params.maxContradictions;
  const completedBlocks = completedContradict <= params.maxContradictions;
  const observed = `${inLog ? "log" : ""}${inLog && completed ? "+" : ""}${completed ? "completed" : ""}`;

  if (logBlocks && completedBlocks) {
    const shares = bestOf(inLog, completed);
    const contradict = logContradict + completedContradict;
    const chain = handsOff(index, questId, target) && (index.quests.get(target)?.required.has(questId) ?? false);
    const leadIn = isLeadIn(index, questId, target);
    // X hands off to one zone copy of a bundle: that copy is the next step, its siblings are no
    // separate relation (and nextQuestInChain holds a single quest).
    const handsOffToSibling = bundleOf(index, target).some((member) => member !== target && handsOff(index, questId, member));
    if (chain || leadIn) {
      const note = `blocked by ${observed}; ${chain ? "chain" : "lead-in"}`;
      edges.push({ questId, field: "nextQuestInChain", target, group: "chain", shares, contradict, note });
      if (leadIn && !chain) edges.push({ questId, field: "breadcrumbForQuestId", target, group: "chain", shares, contradict, note });
    } else if (!handsOffToSibling) {
      const sibling = index.coOffered.has(pairKey(questId, target)) ? "; listed together" : "";
      edges.push({ questId, field: "exclusiveTo", target, group: "exclusiveTo", shares, contradict, note: `blocked by ${observed}${sibling}` });
    }
  } else if (logBlocks && inLog) {
    const field = isLeadIn(index, target, questId) ? "breadcrumbs" : "disabledByQuest";
    // Share of X's offered characters that had already completed Y: high means Y usually comes
    // first, so "Y in log" is more likely a stand-in for an unfinished prerequisite chain.
    const precedes = completedContradict / Math.max(1, offers.characters.size);
    edges.push({ questId, field, target, group: field, shares: inLog, contradict: logContradict, note: `blocked by log only; precedes ${precedes.toFixed(2)}` });
  } else if (completedBlocks && completed) {
    const note = `blocked by completed only; offered while in log by ${logContradict}`;
    edges.push({ questId, field: "availableUntilCompleted", target, group: "availableUntilCompleted", shares: completed, contradict: completedContradict, note });
  }
  return edges;
}

/**
 * A character who has not started a chain is missing every step of it, so the blame for X's block
 * splits between X's direct prerequisite P and P's own prerequisites. Questie lists only P, so a
 * prerequisite suspect that another one requires is dropped and its blame handed over once,
 * split evenly when several kept prerequisites require it.
 */
function withoutAncestors(edges: Edge[], index: OfferIndex): Edge[] {
  const prerequisites = edges.filter((edge) => edge.group === "prerequisite");
  const requires = (edge: Edge, ancestor: Edge) => edge !== ancestor && (index.quests.get(edge.target)?.required.has(ancestor.target) ?? false);
  const dropped = new Set(prerequisites.filter((ancestor) => prerequisites.some((edge) => requires(edge, ancestor))));
  // Two quests that each look required by the other (possible within requiredTolerance) both stay.
  for (const ancestor of [...dropped]) {
    if (!prerequisites.some((edge) => !dropped.has(edge) && requires(edge, ancestor))) dropped.delete(ancestor);
  }
  // Blame computed before any hand-over, so each ancestor's share moves exactly once.
  const ownShares = new Map(prerequisites.map((edge) => [edge, new Map(edge.shares)]));

  for (const ancestor of dropped) {
    const heirs = prerequisites.filter((edge) => !dropped.has(edge) && requires(edge, ancestor));
    for (const heir of heirs) {
      for (const [character, share] of ownShares.get(ancestor)!) {
        const current = heir.shares.get(character);
        const handed = share.share / heirs.length;
        heir.shares.set(character, { ...(current ?? share), share: Math.min(1, (current?.share ?? 0) + handed) });
      }
      // Never having taken an earlier step means never having taken the later one either.
      for (const character of ancestor.untaken ?? []) heir.untaken?.add(character);
    }
  }
  return edges.filter((edge) => !dropped.has(edge));
}

/** Per character, the sharper of the two shares. */
function bestOf(...maps: Array<Map<string, Share> | undefined>): Map<string, Share> {
  const result = new Map<string, Share>();
  for (const map of maps) {
    for (const [character, share] of map ?? []) {
      const best = result.get(character);
      if (!best || share.share > best.share) result.set(character, share);
    }
  }
  return result;
}

function handsOff(index: OfferIndex, from: number, to: number): boolean {
  return index.handoffs.has(`${from}>${to}`);
}

/** `from` was turned in at a giver that offered `to` right after but never offered `from` itself. */
function isLeadIn(index: OfferIndex, from: number, to: number): boolean {
  for (const giver of index.handoffs.get(`${from}>${to}`) ?? []) {
    if (!index.questsByGiver.get(giver)?.has(from)) return true;
  }
  return false;
}
