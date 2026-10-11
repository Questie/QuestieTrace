// Character state at a moment of one episode, tolerant of the addon's sampling lag.
//
// The quest log and completed set are sampled up to ~1s after the event that changed them
// (QuestLog.lua / CompletedQuests.lua SAMPLE_DELAYS), while QUEST_ACCEPTED / QUEST_TURNED_IN
// are recorded when they fire. A giver list re-read right after accepting Y can therefore
// predate the log sample that contains Y. `inLog` and `log` patch the sampled log with the
// quest events of the last `slack` seconds so both sides of a comparison see the same truth.
//
// Some completed-set deltas are catch-up reads that add hundreds of old completions at once.
// Their timestamps are meaningless: those quests count as completed from the session start.

import { EpisodeTimeline } from "../../core/timeline";
import type { QuestEpisode, QuestEvent, SessionTime } from "../../core/types";
import type { AbsenceParams } from "./params";

const LEAVES_LOG: ReadonlySet<QuestEvent["kind"]> = new Set(["turnedIn", "abandoned", "removed"]);

export class EpisodeStates {
  /** The episode with catch-up deltas folded into the initial completed set. */
  readonly episode: QuestEpisode;
  readonly timeline: EpisodeTimeline;
  private readonly slack: number;
  private readonly events: QuestEvent[];
  private readonly eventTimes: number[];
  /** Ascending times at which the completed set may change; the count at or before t versions it. */
  private readonly completionChangeTimes: number[];
  private readonly completedByKey = new Map<string, ReadonlySet<number>>();

  constructor(raw: QuestEpisode, params: Pick<AbsenceParams, "stateSlack" | "catchUpBatch">) {
    const episode = withoutCatchUps(raw, params.catchUpBatch);
    this.episode = episode;
    this.slack = params.stateSlack;
    this.timeline = new EpisodeTimeline(episode);
    this.events = [...episode.questEvents].sort((a, b) => a.t - b.t);
    this.eventTimes = this.events.map((event) => event.t);
    this.completionChangeTimes = [
      ...(episode.completed?.changes ?? []).map((change) => change.t),
      ...this.events.filter((event) => event.kind === "turnedIn").map((event) => event.t),
    ].sort((a, b) => a - b);
  }

  isCompleted(questId: number, t: SessionTime): boolean {
    return this.completed(t).has(questId);
  }

  /** Equal for two times with no completion change between them. */
  private completedVersion(t: SessionTime): number {
    return countAtOrBefore(this.completionChangeTimes, t);
  }

  /**
   * The completed set at t, allowing for sampling lag: quests the completed-set delta reports
   * within `slack` after t count as already completed, unless a turn-in event shows they were
   * completed after t. Quests completed as a side effect (Forever completes every zone copy of
   * a quest together) only ever arrive through the delayed delta.
   */
  completed(t: SessionTime): ReadonlySet<number> {
    const version = this.completedVersion(t + this.slack);
    const key = `${version}|${this.turnInsWithin(t).join(",")}`;
    let set = this.completedByKey.get(key);
    if (!set) {
      const result = this.timeline.completedAt(t + this.slack);
      for (const questId of this.turnInsWithin(t)) result.delete(questId);
      set = result;
      this.completedByKey.set(key, set);
    }
    return set;
  }

  /** Quests turned in during (t, t + slack]. */
  private turnInsWithin(t: SessionTime): number[] {
    const result: number[] = [];
    for (let i = countAtOrBefore(this.eventTimes, t); i < this.events.length && this.events[i].t <= t + this.slack; i++) {
      if (this.events[i].kind === "turnedIn") result.push(this.events[i].questId);
    }
    return result;
  }

  /** The quest log at t, corrected by accept/leave events of the last `slack` seconds. */
  log(t: SessionTime): Set<number> {
    const result = new Set(this.timeline.logAt(t));
    for (const [questId, inLog] of this.recentLogEvents(t)) {
      if (inLog) result.add(questId);
      else result.delete(questId);
    }
    return result;
  }

  inLog(questId: number, t: SessionTime): boolean {
    const recent = this.recentLogEvents(t).get(questId);
    return recent ?? this.timeline.isInLogAt(questId, t);
  }

  /** Any quest event at all within (from, to]. */
  anyEventWithin(from: SessionTime, to: SessionTime): boolean {
    return countAtOrBefore(this.eventTimes, to) > countAtOrBefore(this.eventTimes, from);
  }

  /** Any quest event (dialog, accept, turn-in, ...) for the quest within [from, to]. */
  touched(questId: number, from: SessionTime, to: SessionTime): boolean {
    for (let i = countAtOrBefore(this.eventTimes, from - 1e-9); i < this.events.length; i++) {
      const event = this.events[i];
      if (event.t > to) break;
      if (event.questId === questId) return true;
    }
    return false;
  }

  /** Per quest, whether its last accept/leave event in (t - slack, t] put it in the log. */
  private recentLogEvents(t: SessionTime): Map<number, boolean> {
    const result = new Map<number, boolean>();
    const end = countAtOrBefore(this.eventTimes, t);
    for (let i = countAtOrBefore(this.eventTimes, t - this.slack); i < end; i++) {
      const event = this.events[i];
      if (event.kind === "accepted") result.set(event.questId, true);
      else if (LEAVES_LOG.has(event.kind)) result.set(event.questId, false);
    }
    return result;
  }
}

/**
 * Moves the quests of catch-up deltas (more than `limit` additions) into the initial completed
 * set, except quests with their own events this session, whose completion time the events tell.
 */
export function withoutCatchUps(episode: QuestEpisode, limit: number): QuestEpisode {
  const completed = episode.completed;
  if (!completed || !completed.changes.some((change) => (change.add?.length ?? 0) > limit)) return episode;
  const active = new Set(episode.questEvents.map((event) => event.questId));
  const initial = new Set(completed.initial);
  const changes: typeof completed.changes = [];
  for (const change of completed.changes) {
    if ((change.add?.length ?? 0) <= limit) {
      changes.push(change);
      continue;
    }
    for (const questId of change.add!) if (!active.has(questId)) initial.add(questId);
    const add = change.add!.filter((questId) => active.has(questId));
    if (add.length > 0 || change.remove) changes.push({ ...change, add });
  }
  return { ...episode, completed: { ...completed, initial: [...initial], changes } };
}

/** Number of entries `<= t` in an ascending array. */
export function countAtOrBefore(times: readonly number[], t: number): number {
  let lo = 0;
  let hi = times.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (times[mid] <= t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}
