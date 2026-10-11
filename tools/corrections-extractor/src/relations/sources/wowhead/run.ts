// `npm run relations -- source:wowhead`: Wowhead quest-relation surfaces -> candidates/wowhead-<surface>.json
//
// Reads English quest pages from scraper-questie's Forever raw cache (data/raw/forever.db), and the
// Era cache (data/raw/classic.db) only to tell which claims Forever pages inherited from Era.
// Both are opened read-only. Pipeline: parse.ts (HTML -> facts), interpret.ts (facts -> claims),
// scores.ts (provenance, base scores), aggregate.ts (claims -> candidates).
//
// What the Forever cache holds (2026-10 snapshot): pages fetched from Wowhead's Forever
// environment only render Series and Storyline panels. Requires / Requires Any / Requires In
// Progress / Unlocks / Disables appear only on Era pages the scraper inherited and never
// refetched, so those surfaces are Era data and are only "covered" on those pages.

import { resolve } from "path";
import { writeCandidates, writeJsonAtomic } from "../../core/io";
import { paths } from "../../core/paths";
import type { CandidateFile, RelationCandidate } from "../../core/types";
import { aggregateClaims, type PageClaim, type SeriesPositions } from "./aggregate";
import { claimsFromPage, SURFACES, type QuestLookup, type Surface } from "./interpret";
import { openRawCache, questPages } from "./pages";
import { isPlaceholderName, mergeQuestRecords, parseQuestPage, type QuestPageFacts } from "./parse";
import { claimKey, provenanceOf, type Provenance } from "./scores";

export async function run(_args: string[]): Promise<void> {
  const foreverPath = resolve(paths.scraperDir, "data/raw/forever.db");
  const eraPath = resolve(paths.scraperDir, "data/raw/classic.db");

  const pages = readPages(foreverPath);
  if (pages.length === 0) throw new Error(`${foreverPath} has no English quest pages.`);
  const quests = mergeQuestRecords(pages);
  const eraClaimKeys = readEraClaimKeys(eraPath, quests);

  // Inherited Era pages link Era-only quests (e.g. Season of Discovery ids) that Forever does not
  // have; every registered Forever quest has a page, so a page is the existence check.
  // Unshipped placeholders (<NYI>, <TXT>) have pages but can never be taken.
  const foreverQuests = new Set(pages.filter((page) => !isPlaceholderName(quests.get(page.questId)?.name)).map((page) => page.questId));
  let droppedUnknownQuest = 0;
  const claims: PageClaim[] = [];
  const seriesPositions: SeriesPositions = new Map();
  for (const page of pages) {
    for (const claim of claimsFromPage(page, quests)) {
      if (!foreverQuests.has(claim.questId) || !foreverQuests.has(claim.target)) {
        droppedUnknownQuest++;
        continue;
      }
      const provenance = provenanceOf(page.environment, claimKey(claim), eraClaimKeys);
      claims.push({ ...claim, provenance, environment: page.environment, envChange: page.envChange });
    }
    if (page.series.length > 0) seriesPositions.set(page.questId, rowPositions(page));
  }

  const candidates = aggregateClaims(claims, seriesPositions);
  const generatedAt = new Date().toISOString();
  const pageCounts = countBy(pages, (page) => page.environment);
  const summary: Record<string, unknown> = {};

  for (const surface of SURFACES) {
    const covered = pages.filter((page) => foreverQuests.has(page.questId) && coversSurface(page, surface)).map((page) => page.questId);
    const list = candidates.get(surface) ?? [];
    const file: CandidateFile = {
      signal: `wowhead-${surface}`,
      generatedAt,
      inputCount: pages.length,
      params: {
        foreverDb: foreverPath,
        eraDb: eraPath,
        pages: pageCounts,
        coverage: COVERAGE[surface],
        support: "Wowhead pages showing the edge (renderings of the same data, not independent observations)",
        contradict: "Series only: pages whose Series shows both quests, but not directly one after the other",
        note: "note = surface, field, shape and provenance of the best claim (see sources/wowhead/scores.ts)",
      },
      coveredQuestIds: covered,
      candidates: list,
    };
    const path = writeCandidates(file);
    summary[surface] = { covered: covered.length, candidates: list.length, byField: countBy(list, (c) => c.field), byProvenance: countBy(list, provenanceTag) };
    console.log(`${file.signal.padEnd(32)} ${String(list.length).padStart(6)} candidates, ${String(covered.length).padStart(5)} covered -> ${path}`);
  }

  const report = {
    generatedAt,
    foreverDb: foreverPath,
    eraDb: eraPath,
    pages: pageCounts,
    droppedClaimsForUnknownOrPlaceholderQuests: droppedUnknownQuest,
    // Series rows whose markup did not parse; they break adjacency, so check this stays 0.
    unparsedSeriesRows: countBy(
      pages.flatMap((page) => page.series.filter((row) => row.unparsed).map(() => page.environment)),
      (environment) => environment,
    ),
    envChange: countBy(pages, (page) => `${page.environment}:${page.envChange ?? "none"}`),
    pagesWithSurface: Object.fromEntries(
      SURFACES.map((surface) => [surface, countBy(pages.filter((page) => hasSurface(page, surface)), (page) => page.environment)]),
    ),
    candidates: summary,
  };
  writeJsonAtomic(resolve(paths.reportsDir, "wowhead-inventory.json"), report);
  console.log(`pages: ${JSON.stringify(pageCounts)}; ${droppedUnknownQuest} claims dropped (quest not in Forever or a placeholder)`);
}

const COVERAGE: Record<Surface, string> = {
  series: "every parsed page (Forever and inherited Era pages both render Series)",
  storyline: "pages fetched from Wowhead Forever (Era pages have no Storyline panel)",
  requires: "inherited Era pages only (Forever pages never render this panel)",
  "requires-any": "inherited Era pages only (Forever pages never render this panel)",
  "requires-in-progress": "inherited Era pages only (Forever pages never render this panel)",
  unlocks: "inherited Era pages only; candidates sit on the unlocked quests, so recall is not meaningful",
  disables: "inherited Era pages only; candidates sit on the disabled quests, so recall is not meaningful",
};

const PANEL_OF: Partial<Record<Surface, keyof QuestPageFacts["panels"]>> = {
  requires: "Requires",
  "requires-any": "Requires Any",
  "requires-in-progress": "Requires In Progress",
  unlocks: "Unlocks",
  disables: "Disables",
};

function coversSurface(page: QuestPageFacts, surface: Surface): boolean {
  if (surface === "series") return true;
  if (surface === "storyline") return page.environment === "forever";
  return page.environment !== "forever";
}

function hasSurface(page: QuestPageFacts, surface: Surface): boolean {
  if (surface === "series") return page.series.length > 0;
  if (surface === "storyline") return page.storyline !== undefined;
  return page.panels[PANEL_OF[surface]!] !== undefined;
}

function readPages(path: string): QuestPageFacts[] {
  const db = openRawCache(path);
  try {
    return [...questPages(db)].map((stored) => parseQuestPage(stored.questId, stored.body));
  } finally {
    db.close();
  }
}

/** Every claim the Era cache's pages produce, keyed like ./scores.ts claimKey. */
function readEraClaimKeys(path: string, quests: QuestLookup): Set<string> {
  const db = openRawCache(path);
  const keys = new Set<string>();
  try {
    for (const stored of questPages(db)) {
      for (const claim of claimsFromPage(parseQuestPage(stored.questId, stored.body), quests)) keys.add(claimKey(claim));
    }
  } finally {
    db.close();
  }
  if (keys.size === 0) throw new Error(`${path} produced no claims; is it the Era raw cache?`);
  return keys;
}

function rowPositions(page: QuestPageFacts): Map<number, number> {
  const positions = new Map<number, number>();
  page.series.forEach((row, position) => {
    for (const member of row.members) positions.set(member.questId, position);
  });
  return positions;
}

function provenanceTag(candidate: RelationCandidate): Provenance | "unknown" {
  return (/provenance=([a-z-]+)/.exec(candidate.note ?? "")?.[1] as Provenance | undefined) ?? "unknown";
}

function countBy<T>(items: Iterable<T>, key: (item: T) => string): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const item of items) {
    const k = key(item);
    counts[k] = (counts[k] ?? 0) + 1;
  }
  return counts;
}
