// Merges the copies of one recording session that appear in several submissions.
//
// Exports used to be cumulative and always include the in-progress `currentSession`, so one
// session shows up in many submissions, growing as play continues. Sessions are identified by
// their capture-start clocks (sessionKey); the copy with the most events is the most complete.

import type { DistilledSession } from "./types";

export interface SubmissionInfo {
  submissionId: string;
  contributorId: string;
  receivedAt: string;
}

export interface MergedSession {
  /** The most complete copy. */
  session: DistilledSession;
  /** Contributor of the earliest submission containing the session. */
  contributorId: string;
  contributorIds: Set<string>;
  submissionIds: Set<string>;
  firstReceivedAt: string;
  /** Submission the kept copy came from, for deterministic tie-breaks. */
  bestSubmissionId: string;
}

/** Whether copy `a` (from submission `aId`) should replace the kept copy `b`. */
function isBetter(a: DistilledSession, aId: string, b: DistilledSession, bId: string): boolean {
  if (a.eventCount !== b.eventCount) return a.eventCount > b.eventCount;
  if (a.saved !== b.saved) return a.saved;
  return aId < bId;
}

export class SessionMerger {
  readonly merged = new Map<string, MergedSession>();
  copies = 0;

  add(submission: SubmissionInfo, session: DistilledSession): void {
    this.copies++;
    const existing = this.merged.get(session.key);
    if (!existing) {
      this.merged.set(session.key, {
        session,
        contributorId: submission.contributorId,
        contributorIds: new Set([submission.contributorId]),
        submissionIds: new Set([submission.submissionId]),
        firstReceivedAt: submission.receivedAt,
        bestSubmissionId: submission.submissionId,
      });
      return;
    }

    existing.submissionIds.add(submission.submissionId);
    existing.contributorIds.add(submission.contributorId);
    if (submission.receivedAt < existing.firstReceivedAt) {
      existing.firstReceivedAt = submission.receivedAt;
      existing.contributorId = submission.contributorId;
    }
    // A saved copy has the session name even when an unsaved copy has as many events.
    const sessionName = existing.session.episode.sessionName ?? session.episode.sessionName;
    if (isBetter(session, submission.submissionId, existing.session, existing.bestSubmissionId)) {
      existing.session = session;
      existing.bestSubmissionId = submission.submissionId;
    }
    if (sessionName) existing.session.episode.sessionName = sessionName;
  }
}
