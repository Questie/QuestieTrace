// relations-meta.json: the summary of a combine run that ships next to the Lua module. The trace
// analyzer shows it with the module, and it marks a complete run the way the field extractor's
// meta.json does: combine removes it first and writes it last. It has its own name so the two
// pipelines, which share output/, never overwrite each other's summary.

import { isForeverInterface } from "../core/eligibility";
import type { QuestEpisode } from "../core/types";
import { isWritten, type QuestView, type Status } from "./views";

/** One line for readers of the deliverables; METRIC_CAVEATS and the review report have the long form. */
export const LEAKAGE_CAVEAT =
  "Held-out metrics are optimistic: every signal calibrated its scores on the same Questie ground truth the metrics use (see relations-review.md).";

export interface RelationsMeta {
  generatedAt: string;
  /** Acceptance threshold: relations at or above it are in the Lua module. */
  minScore: number;
  /** Episodes in episodes.jsonl that the trace signals read (Forever and unversioned builds), and the characters behind them. */
  episodeCount: number;
  characterCount: number;
  /** What foreverQuestRelationTraces.lua holds. A breadcrumb or exclusive pair counts once per quest it is written to. */
  lua: { quests: number; relations: number };
  /** Accepted relations on Forever-new quests with authored relations, by status against the authored data. */
  authored: Record<Status, number>;
  /** Candidate files the model used. */
  inputs: Array<{ signal: string; generatedAt: string; inputCount: number; candidates: number; covered: number }>;
  /** Shown to reviewers as context only (Wowhead), never scored. */
  contextOnly: string[];
  caveat: string;
}

/** Counts the same episodes the signals read (see isForeverInterface), and their distinct characters. */
export async function countEpisodes(episodes: AsyncIterable<QuestEpisode>): Promise<{ episodes: number; characters: number }> {
  let count = 0;
  const characters = new Set<string>();
  for await (const episode of episodes) {
    if (!isForeverInterface(episode.interfaceVersion)) continue;
    count++;
    characters.add(episode.characterKey);
  }
  return { episodes: count, characters: characters.size };
}

/** Relation values in the Lua records: each id of a list field, one per scalar field. */
export function luaRelationCount(records: ReadonlyMap<number, Record<string, unknown>>): number {
  let count = 0;
  for (const record of records.values()) {
    for (const value of Object.values(record)) count += Array.isArray(value) ? value.length : 1;
  }
  return count;
}

export function authoredStatusCounts(views: Iterable<QuestView>): Record<Status, number> {
  const counts: Record<Status, number> = { agree: 0, new: 0, conflict: 0 };
  for (const view of views) {
    if (view.domain !== "forever" || view.reference !== "authored") continue;
    for (const edge of view.edges) if (isWritten(edge) && edge.status) counts[edge.status]++;
  }
  return counts;
}
