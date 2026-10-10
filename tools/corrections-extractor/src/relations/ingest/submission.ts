// Phase 1 of ingest, per submission: decode its export string(s) and distill every session.
// The result is cached in .relations/ingest-cache/<submissionId>.json so a re-run only
// decodes submissions it has not seen.

import { createHash } from "crypto";
import { existsSync, readFileSync } from "fs";
import { basename, dirname, relative, resolve } from "path";
import { fileURLToPath } from "url";
import { paths } from "../core/paths";
import { readJson } from "../core/io";
import { decodeExportString } from "./decode";
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
 * module processSubmission runs (decode, CBOR, distill and its parameters, contracts) is hashed.
 */
export const CACHE_VERSION = cacheVersionOf(pipelineSources(here), dirname(here));

interface SubmissionFile {
  id?: unknown;
  contributor_id?: unknown;
  received_at?: unknown;
  export_string?: unknown;
}

/** trace-data names files `<received stamp>_<submission id>.json`. */
export function submissionIdFromPath(path: string): string {
  const name = basename(path, ".json");
  return name.slice(name.lastIndexOf("_") + 1);
}

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
  const fallbackId = submissionIdFromPath(path);
  let raw: SubmissionFile;
  try {
    raw = JSON.parse(readFileSync(path, "utf8")) as SubmissionFile;
  } catch {
    // JSON.parse messages quote the input, and submissions carry free-text fields.
    return failed(fallbackId, "", "", "unreadable submission JSON");
  }
  const submissionId = typeof raw.id === "string" && raw.id.length > 0 ? raw.id : fallbackId;
  const contributorId = typeof raw.contributor_id === "string" ? raw.contributor_id : "";
  const receivedAt = typeof raw.received_at === "string" ? raw.received_at : "";
  if (typeof raw.export_string !== "string") return failed(submissionId, contributorId, receivedAt, "no export_string");

  try {
    const traces = decodeExportString(raw.export_string);
    const sessions = traces.flatMap((trace) => (Array.isArray(trace.sessions) ? trace.sessions : []).map(distillSession));
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
