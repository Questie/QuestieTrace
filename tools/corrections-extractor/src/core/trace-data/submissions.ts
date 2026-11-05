// Reads a trace-data checkout: submissions/<yyyy-mm>/<stamp>_<id>.json, each holding one or
// more addon exports in `export_string`.
//
// Only pseudonymous fields leave this module: the submission id, the contributor id and the
// receive time. Nothing else in a submission file (such as its free-text `nickname`) is read.

import { existsSync, readdirSync, readFileSync } from "fs";
import { basename, dirname, resolve } from "path";
import { fileURLToPath } from "url";
import type { SessionRecord, TraceFile } from "../types";
import { decodeExportString } from "./decode";

const addonsDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../../..");

/** TRACE_DATA_DIR, else the trace-data checkout next to the QuestieTrace checkout. */
export function defaultTraceDataDir(): string {
  const fromEnv = process.env.TRACE_DATA_DIR;
  return fromEnv && fromEnv.length > 0 ? resolve(fromEnv) : resolve(addonsDir, "trace-data");
}

export interface SubmissionFile {
  path: string;
  /** The id in the file name. */
  id: string;
}

/** trace-data names files `<received stamp>_<submission id>.json`. */
function submissionIdFromPath(path: string): string {
  const name = basename(path, ".json");
  return name.slice(name.lastIndexOf("_") + 1);
}

/** Every submission file in the checkout, oldest first: file names start with the receive stamp. */
export function listSubmissionFiles(traceDataDir: string): SubmissionFile[] {
  const dir = resolve(traceDataDir, "submissions");
  if (!existsSync(dir)) throw new Error(`${dir} does not exist. Set TRACE_DATA_DIR to a trace-data checkout.`);
  const files: SubmissionFile[] = [];
  for (const month of readdirSync(dir, { withFileTypes: true })) {
    if (!month.isDirectory()) continue;
    for (const name of readdirSync(resolve(dir, month.name))) {
      if (name.endsWith(".json")) files.push({ path: resolve(dir, month.name, name), id: submissionIdFromPath(name) });
    }
  }
  if (files.length === 0) throw new Error(`${dir} has no submissions`);
  return files.sort((a, b) => (a.path < b.path ? -1 : 1));
}

/** One submission, decoded. When it cannot be read, `error` says why and `traces` is empty. */
export interface Submission {
  /** The file's `id`, else the id in its file name. */
  submissionId: string;
  /** Pseudonymous contributor id; "" when missing. */
  contributorId: string;
  /** ISO time trace-data received the submission; "" when missing. */
  receivedAt: string;
  /** One per export in `export_string` (usually one), each with a `sessions` array. */
  traces: TraceFile[];
  /** Why each export of `export_string` that could not be decoded was dropped. */
  droppedExports: string[];
  error?: string;
}

interface RawSubmission {
  id?: unknown;
  contributor_id?: unknown;
  received_at?: unknown;
  export_string?: unknown;
}

function parseJson(path: string): RawSubmission | undefined {
  try {
    const raw: unknown = JSON.parse(readFileSync(path, "utf8"));
    return raw !== null && typeof raw === "object" ? raw : undefined;
  } catch {
    // Never pass JSON.parse's message on: it quotes the input, and submissions carry free-text fields.
    return undefined;
  }
}

/** Sessions without events or functions cannot be read by any consumer; they are dropped. */
function isSessionRecord(session: unknown): session is SessionRecord {
  if (session === null || typeof session !== "object") return false;
  const { events, functions } = session as Record<string, unknown>;
  return Array.isArray(events) && functions !== null && typeof functions === "object" && !Array.isArray(functions);
}

export function readSubmission(path: string): Submission {
  const raw = parseJson(path);
  const submissionId = typeof raw?.id === "string" && raw.id.length > 0 ? raw.id : submissionIdFromPath(path);
  const contributorId = typeof raw?.contributor_id === "string" ? raw.contributor_id : "";
  const receivedAt = typeof raw?.received_at === "string" ? raw.received_at : "";
  const failed = (error: string): Submission => ({ submissionId, contributorId, receivedAt, traces: [], droppedExports: [], error });
  if (!raw) return failed("unreadable submission JSON");
  if (typeof raw.export_string !== "string") return failed("no export_string");

  try {
    const { traces, dropped } = decodeExportString(raw.export_string);
    return {
      submissionId,
      contributorId,
      receivedAt,
      traces: traces.map((trace) => ({ ...trace, sessions: Array.isArray(trace.sessions) ? trace.sessions.filter(isSessionRecord) : [] })),
      droppedExports: dropped,
    };
  } catch (e) {
    return failed(e instanceof Error ? e.message : String(e));
  }
}
