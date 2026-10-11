// Merges per-page claims (./interpret.ts) into one RelationCandidate per (quest, field, target).
//
// Every quest in a Series shows the whole Series, so one edge is usually claimed by several
// pages. Those pages render the same Wowhead data, so they are witnesses, not independent
// observations: `support` counts pages showing the edge, `contradict` counts pages whose Series
// shows both quests but not directly one after the other.

import type { RelationCandidate, RelationField } from "../../core/types";
import type { Claim, ClaimShape, Surface } from "./interpret";
import type { PageEnvironment } from "./parse";
import { baseScore, claimKey, type Provenance } from "./scores";

export interface PageClaim extends Claim {
  provenance: Provenance;
  environment: PageEnvironment;
  /** envChange.status of the page's own quest. */
  envChange?: string;
}

/** Page id -> (quest id -> Series row position) for every page with a Series. Used to count contradicting pages. */
export type SeriesPositions = Map<number, ReadonlyMap<number, number>>;

const MAX_EVIDENCE = 5;
const PROVENANCE_ORDER: Provenance[] = ["forever-new-edge", "forever-era-edge", "era-page"];

export function aggregateClaims(claims: PageClaim[], seriesPositions: SeriesPositions): Map<Surface, RelationCandidate[]> {
  const groups = new Map<string, PageClaim[]>();
  for (const claim of claims) {
    const key = claimKey(claim);
    const group = groups.get(key);
    if (group) group.push(claim);
    else groups.set(key, [claim]);
  }

  const pagesByQuest = indexPagesByQuest(seriesPositions);
  const bySurface = new Map<Surface, RelationCandidate[]>();
  for (const group of groups.values()) {
    const candidate = toCandidate(group, seriesPositions, pagesByQuest);
    const list = bySurface.get(group[0].surface);
    if (list) list.push(candidate);
    else bySurface.set(group[0].surface, [candidate]);
  }
  for (const list of bySurface.values()) list.sort((a, b) => a.questId - b.questId || a.field.localeCompare(b.field) || a.target - b.target);
  return bySurface;
}

function toCandidate(group: PageClaim[], seriesPositions: SeriesPositions, pagesByQuest: Map<number, number[]>): RelationCandidate {
  const { surface, questId, field, target } = group[0];
  const ranked = [...group].sort(
    (a, b) => scoreOf(b) - scoreOf(a) || PROVENANCE_ORDER.indexOf(a.provenance) - PROVENANCE_ORDER.indexOf(b.provenance) || a.pageId - b.pageId,
  );
  const best = ranked[0];

  const pages = new Set(group.map((claim) => claim.pageId));
  const support = pages.size;
  const contradict = best.adjacency ? contradictingPages(best.adjacency, seriesPositions, pagesByQuest) : 0;
  // Pages that show the two quests in a different order or with steps between them dilute the edge.
  const score = round(scoreOf(best) * (support / (support + contradict)));

  const evidence: RelationCandidate["evidence"] = [];
  const seen = new Set<number>();
  for (const claim of [...group].sort(byFreshestPage)) {
    if (seen.has(claim.pageId)) continue;
    seen.add(claim.pageId);
    evidence.push({ ref: `wowhead:quest/${claim.pageId}`, text: `${describePage(claim)}: ${claim.text}` });
    if (evidence.length >= MAX_EVIDENCE) break;
  }

  return { questId, field, target, score, support, contradict, evidence, note: noteFor(surface, field, best.shape, best.provenance) };
}

function scoreOf(claim: PageClaim): number {
  return baseScore(claim.surface, claim.field, claim.shape, claim.provenance);
}

/** Machine-readable tags for review and per-provenance scoring. */
function noteFor(surface: Surface, field: RelationField, shape: ClaimShape, provenance: Provenance): string {
  return `surface=${surface} field=${field} shape=${shape} provenance=${provenance}`;
}

function contradictingPages([before, after]: [number, number], seriesPositions: SeriesPositions, pagesByQuest: Map<number, number[]>): number {
  let contradict = 0;
  for (const pageId of pagesByQuest.get(before) ?? []) {
    const positions = seriesPositions.get(pageId)!;
    const a = positions.get(before)!;
    const b = positions.get(after);
    if (b !== undefined && b !== a + 1) contradict++;
  }
  return contradict;
}

function indexPagesByQuest(seriesPositions: SeriesPositions): Map<number, number[]> {
  const index = new Map<number, number[]>();
  for (const [pageId, positions] of seriesPositions) {
    for (const questId of positions.keys()) {
      const pages = index.get(questId);
      if (pages) pages.push(pageId);
      else index.set(questId, [pageId]);
    }
  }
  return index;
}

function byFreshestPage(a: PageClaim, b: PageClaim): number {
  const rank = (claim: PageClaim) => (claim.environment === "forever" ? 0 : 1);
  return rank(a) - rank(b) || a.pageId - b.pageId;
}

function describePage(claim: PageClaim): string {
  const env = claim.environment === "forever" ? "Forever page" : claim.environment === "era" ? "inherited Era page" : "page";
  const status = claim.envChange ? `, envChange ${claim.envChange}` : "";
  const edge = { "forever-new-edge": ", edge not on any Era page", "forever-era-edge": ", edge also on Era pages", "era-page": "" }[claim.provenance];
  return `${env} ${claim.pageId}${status}${edge}`;
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}
