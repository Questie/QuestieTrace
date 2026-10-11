// Phase 1 of ingest, per submission: decode its export string(s) and distill every session.
// The result is cached in .relations/ingest-cache/<submissionId>.json so a re-run only
// decodes submissions it has not seen.

import { createHash } from "crypto";
import { existsSync, readFileSync } from "fs";
import { dirname, relative, resolve } from "path";
import { fileURLToPath } from "url";
import { readSubmission } from "../../core/trace-data/submissions";
import { paths } from "../core/paths";
import { readJson } from "../core/io";
import { distillSession } from "./distill";
import type { SubmissionResult } from "./types";

const IMPORT = /\bfrom\s+["'](\.{1,2}\/[^"']+)["']/g;

/** Source text of `entry` and of every local module it imports, directly or not, keyed by path. */
export function pipelineSources(entry: string, read: (path: string) => string = (path) => readFileSync(path, "utf8")): Map<string, string> {
  const sources = new Map<string, string>();
  const visit = (path: string) => {
    if (sources.has(path)) return;
    const text = read(path);
    sources.set(path, text);
    for (const match of text.matchAll(IMPORT)) {
      const base = resolve(dirname(path), match[1].replace(/\.js$/, ""));
      visit(existsSync(`${base}.ts`) ? `${base}.ts` : resolve(base, "index.ts"));
    }
  };
  visit(entry);
  return sources;
}

/** Hash of the sources (paths relative to `root`, so the key is the same on every machine). */
export function cacheVersionOf(sources: ReadonlyMap<string, string>, root: string): string {
  const hash = createHash("sha256");
  for (const [path, text] of [...sources].sort(([a], [b]) => a.localeCompare(b))) hash.update(`${relative(root, path)}\0${text}\0`);
  return hash.digest("hex").slice(0, 16);
}

const here = fileURLToPath(import.meta.url);
/**
 * Cache entries are reused only when written by exactly the code that would write them now: every
 * module processSubmission runs (reader, decode, CBOR, distill and its parameters, contracts) is hashed.
 */
export const CACHE_VERSION = cacheVersionOf(pipelineSources(here), dirname(here));

export function cachePath(submissionId: string): string {
  return resolve(paths.ingestCacheDir, `${submissionId}.json`);
}

export function readCachedResult(submissionId: string): SubmissionResult | undefined {
  try {
    const cached = readJson<SubmissionResult>(cachePath(submissionId), "ingest");
    return cached.cacheVersion === CACHE_VERSION ? cached : undefined;
  } catch {
    return undefined;
  }
}

export function processSubmission(path: string): SubmissionResult {
  const { submissionId, contributorId, receivedAt, traces, error } = readSubmission(path);
  if (error) return failed(submissionId, contributorId, receivedAt, error);

  try {
    const sessions = traces.flatMap((trace) => trace.sessions.map(distillSession));
    return { cacheVersion: CACHE_VERSION, submissionId, contributorId, receivedAt, exports: traces.length, sessions };
  } catch (e) {
    return failed(submissionId, contributorId, receivedAt, message(e));
  }
}

function failed(submissionId: string, contributorId: string, receivedAt: string, error: string): SubmissionResult {
  return { cacheVersion: CACHE_VERSION, submissionId, contributorId, receivedAt, exports: 0, error, sessions: [] };
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
