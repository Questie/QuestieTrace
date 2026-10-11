// Ingest-internal shapes: what one submission distills to before sessions are merged
// across submissions and grouped into characters (see run.ts for the two phases).

import type { GiverRef, QuestEpisode } from "../core/types";

/** A QuestEpisode before merge: identity and ordering fields that need every submission are filled in later. */
export type EpisodeDraft = Omit<QuestEpisode, "contributorId" | "characterKey" | "submissionIds" | "sessionOrder">;

/**
 * A legacy greeting list that only recorded titles. Resolved to quest ids once the
 * (locale, title) -> quest id map from all submissions is known.
 */
export interface PendingGreeting {
  /** Index into `episode.offers`. */
  offer: number;
  available: string[];
  active: string[];
  /** GetNumAvailableQuests; fewer readable titles than this means the list cannot be complete. */
  expected: number;
  /** Set when the list cannot be complete whatever the titles resolve to (see markInFlightAccepts). */
  incomplete?: boolean;
}

/**
 * One (title, quest id) pairing seen in a dialog, gossip list or the quest log. Only used to
 * resolve legacy greeting titles; never written to episodes.
 */
export interface TitleObservation {
  locale: string;
  /** The observing character's faction: same-titled Alliance and Horde quests are common. */
  faction?: string;
  title: string;
  questId: number;
  /** "npc:123" / "object:456" when the pairing was seen at a known giver. */
  giver?: string;
}

/** Dialog reads distill could not use, counted per session. */
export interface DistillStats {
  /** QUEST_DETAIL whose quest id never showed in GetQuestID nor in a following QUEST_ACCEPTED. */
  detailUnresolved: number;
  /** QUEST_PROGRESS/QUEST_COMPLETE likewise, checked against QUEST_TURNED_IN. */
  progressUnresolved: number;
  greetingWithoutCount: number;
  gossipWithoutRead: number;
  /** Item-started details; Forever records no item id for them, so they have no giver. */
  itemStartedDetails: number;
  /** Dialog quest ids taken from the accept/turn-in that answered the dialog. */
  borrowedQuestIds: number;
  /** GOSSIP_SHOWs without their own read whose non-empty lists were last read at another giver (never complete). */
  gossipInheritedFromOtherGiver: number;
  /** Empty completed-quest reads (bulk removal + re-add) collapsed out of the timeline. */
  completedReadGaps: number;
}

export function emptyDistillStats(): DistillStats {
  return { detailUnresolved: 0, progressUnresolved: 0, greetingWithoutCount: 0, gossipWithoutRead: 0, itemStartedDetails: 0, borrowedQuestIds: 0, gossipInheritedFromOtherGiver: 0, completedReadGaps: 0 };
}

/** One session of one submission, distilled. */
export interface DistilledSession {
  /** Stable episode key (see sessionKey). */
  key: string;
  /** How complete this copy is; the copy with the most events wins a merge. */
  eventCount: number;
  /** Stopped/saved sessions carry a name; in-progress `currentSession` copies do not. */
  saved: boolean;
  /** GetServerTime at capture start (epoch seconds). Orders a contributor's sessions; never written to episodes. */
  startServerTime?: number;
  episode: EpisodeDraft;
  pendingGreetings: PendingGreeting[];
  titles: TitleObservation[];
  stats: DistillStats;
}

/** Per-submission cache entry (.relations/ingest-cache/<submissionId>.json). */
export interface SubmissionResult {
  cacheVersion: string;
  submissionId: string;
  contributorId: string;
  receivedAt: string;
  exports: number;
  /** Set when the submission could not be decoded; `sessions` is then empty. */
  error?: string;
  sessions: DistilledSession[];
}

export function giverKey(giver: GiverRef): string {
  return `${giver.kind}:${giver.id}`;
}
