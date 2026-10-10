// Locations of the relationship pipeline's inputs and its (gitignored) cache.
// Every path can be overridden with an environment variable, so tests and
// alternate checkouts never need code changes.

import { dirname, resolve } from "path";
import { fileURLToPath } from "url";

const toolDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const repoRoot = resolve(toolDir, "../..");
const projectsDir = resolve(repoRoot, "../..");

function fromEnv(name: string, fallback: string): string {
  const value = process.env[name];
  return value && value.length > 0 ? resolve(value) : fallback;
}

/** Root of the gitignored cache every area writes to. */
export const relationsDir = fromEnv("RELATIONS_DIR", resolve(toolDir, ".relations"));

export const paths = {
  relationsDir,
  /** trace-data checkout: submissions/<yyyy-mm>/<stamp>_<id>.json, each holding an export_string. */
  traceDataDir: fromEnv("TRACE_DATA_DIR", resolve(repoRoot, "trace-data")),
  /** QuestieDB checkout (data/ and src/corrections/). */
  questieDbDir: fromEnv("QUESTIEDB_DIR", resolve(repoRoot, "../QuestieDB")),
  /** Wowhead scrape (data/raw/forever.db, data/parsed/forever.db). Open read-only: the scraper may be running. */
  scraperDir: fromEnv("SCRAPER_DIR", resolve(projectsDir, "scraper-questie")),

  /** One QuestEpisode per line, deduplicated across submissions. */
  episodes: resolve(relationsDir, "episodes.jsonl"),
  /** Ingest's private incremental cache (per-submission intermediate results). */
  ingestCacheDir: resolve(relationsDir, "ingest-cache"),
  catalog: resolve(relationsDir, "catalog.json"),
  groundTruth: resolve(relationsDir, "groundtruth.json"),
  /** candidates/<signal>.json, one CandidateFile per signal or source. */
  candidatesDir: resolve(relationsDir, "candidates"),
  /** Human-readable reports (scores, review lists). */
  reportsDir: resolve(relationsDir, "reports"),
} as const;

export function candidatePath(signal: string): string {
  return resolve(paths.candidatesDir, `${signal}.json`);
}
