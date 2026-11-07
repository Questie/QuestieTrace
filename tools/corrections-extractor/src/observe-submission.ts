// Reads one submission and runs every observer over each of its sessions. Pure, so it can
// run in a worker thread; the main thread decides which sessions to keep (see
// extract-corrections.ts).

import { sessionKey } from "./core/trace-data/session-key";
import { readSubmission } from "./core/trace-data/submissions";
import { observeSession, type MAX_IDS, type StagedSession } from "./extract";
import { observeLookupSession, type StagedLookupSession } from "./extract/lookups";

export interface ObserveRequest {
  path: string;
  maxIds: Partial<typeof MAX_IDS>;
}

export interface ObservedSession {
  /** Undefined when the session cannot be matched with its copies. */
  key: string | undefined;
  extract?: StagedSession;
  lookup?: StagedLookupSession;
  /** Name of the error an observer threw; the run aborts if this session is used. */
  error?: string;
}

export interface ObservedSubmission {
  submissionId: string;
  exportCount: number;
  droppedExports: string[];
  sessions: ObservedSession[];
  error?: string;
}

/**
 * `usedKeys` are sessions already folded in from newer submissions: their copies here are
 * duplicates, so they are not observed.
 */
export function observeSubmission({ path, maxIds }: ObserveRequest, usedKeys: ReadonlySet<string> = new Set()): ObservedSubmission {
  const submission = readSubmission(path);
  // Only the first copy of a session in a submission is used, so later ones are not observed.
  const seen = new Set<string>();
  const sessions = submission.traces.flatMap((trace) => trace.sessions).map((session): ObservedSession => {
    const key = sessionKey(session);
    if (key !== undefined && (usedKeys.has(key) || seen.has(key))) return { key };
    if (key !== undefined) seen.add(key);
    try {
      return { key, extract: observeSession(session, maxIds), lookup: observeLookupSession(session) };
    } catch (e) {
      // Only the error's name: its message can quote trace data.
      return { key, error: e instanceof Error ? e.name : typeof e };
    }
  });
  return {
    submissionId: submission.submissionId,
    exportCount: submission.traces.length,
    droppedExports: submission.droppedExports,
    sessions,
    error: submission.error,
  };
}
