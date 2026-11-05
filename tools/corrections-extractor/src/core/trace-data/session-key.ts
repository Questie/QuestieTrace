// Identity of one recording session across trace files and submissions.
//
// Exports are cumulative and always include the in-progress session, so one session shows up
// in many submissions, growing as play continues. Its capture-start clocks never change, so
// every copy has the same key.
//
// Which copy to keep: the extractor keeps the newest (extract-corrections.ts), ingest keeps the
// one with the most events (relations/ingest/merge.ts). They pick the same copy in practice: a
// newer copy only ever extends an older one. Measured on trace-data in 2026-10, the newest copy
// had fewer events than another copy in 0 of 1,339 sessions seen in several submissions, and
// within a submission holding several exports the first copy was never the smaller one.

import { createHash } from "crypto";
import type { SessionRecord } from "../types";

/**
 * Stable pseudonymous id of a recording session: a hash of its two capture-start clocks.
 * Undefined when either clock is missing; such a session cannot be matched with its copies,
 * so callers must treat it as unique.
 */
export function sessionKey(session: Pick<SessionRecord, "startedAt" | "startedAtPrecise">): string | undefined {
  const { startedAt, startedAtPrecise } = session;
  if (!Number.isFinite(startedAt) || !Number.isFinite(startedAtPrecise)) return undefined;
  return createHash("sha256").update(`${startedAt}|${startedAtPrecise}`).digest("hex").slice(0, 16);
}
