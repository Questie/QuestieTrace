// Point-in-time lookups over one QuestEpisode: "at time t, was quest X completed /
// in the log, and what level was the character?". Every signal needs these, so
// they live in core and are built once per episode.

import type { QuestEpisode, SessionTime, TimedValue } from "./types";

/** Index of the last entry with `time <= t`, or -1. `times` must be ascending. */
function lastIndexAtOrBefore(times: readonly number[], t: number): number {
  let lo = 0;
  let hi = times.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (times[mid] <= t) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

interface Toggle {
  t: SessionTime;
  completed: boolean;
}

export interface TimelineOptions {
  /**
   * Treat a QUEST_TURNED_IN event as completion from the event's time, even before the
   * completed-quest delta shows it (the delta is sampled with a short delay). Default true.
   * Repeatable quests are never flagged completed by the client, so this over-reports them.
   */
  includeTurnIns?: boolean;
}

export class EpisodeTimeline {
  private readonly initialCompleted: ReadonlySet<number>;
  private readonly toggles = new Map<number, Toggle[]>();
  /** Per quest, the toggle times (ascending) parallel to `toggles`, for binary search. */
  private readonly toggleTimes = new Map<number, number[]>();
  private readonly logTimes: number[];
  private readonly logSets: ReadonlySet<number>[];
  private readonly levelTimes: number[];
  private readonly levelValues: number[];

  constructor(
    readonly episode: QuestEpisode,
    options: TimelineOptions = {},
  ) {
    const includeTurnIns = options.includeTurnIns ?? true;
    this.initialCompleted = new Set(episode.completed?.initial ?? []);

    for (const change of episode.completed?.changes ?? []) {
      for (const id of change.add ?? []) this.addToggle(id, { t: change.t, completed: true });
      for (const id of change.remove ?? []) this.addToggle(id, { t: change.t, completed: false });
    }
    if (includeTurnIns) {
      for (const event of episode.questEvents) {
        if (event.kind === "turnedIn") this.addToggle(event.questId, { t: event.t, completed: true });
      }
    }
    for (const [questId, list] of this.toggles) {
      list.sort((a, b) => a.t - b.t);
      this.toggleTimes.set(questId, list.map((toggle) => toggle.t));
    }

    this.logTimes = episode.questLog.map((entry) => entry.t);
    this.logSets = episode.questLog.map((entry) => new Set(entry.v));
    this.levelTimes = episode.levels.map((entry) => entry.t);
    this.levelValues = episode.levels.map((entry) => entry.v);
  }

  private addToggle(questId: number, toggle: Toggle): void {
    const list = this.toggles.get(questId);
    if (list) list.push(toggle);
    else this.toggles.set(questId, [toggle]);
  }

  isCompletedAt(questId: number, t: SessionTime): boolean {
    const list = this.toggles.get(questId);
    if (!list) return this.initialCompleted.has(questId);
    const index = lastIndexAtOrBefore(this.toggleTimes.get(questId)!, t);
    return index === -1 ? this.initialCompleted.has(questId) : list[index].completed;
  }

  /** The full completed set at `t`. Costs O(completed history); prefer isCompletedAt in loops. */
  completedAt(t: SessionTime): Set<number> {
    const result = new Set(this.initialCompleted);
    for (const [questId] of this.toggles) {
      if (this.isCompletedAt(questId, t)) result.add(questId);
      else result.delete(questId);
    }
    return result;
  }

  /**
   * When the quest first became completed in this session: "initial" if it already was at the
   * start, a session time if it became completed later, undefined if never.
   */
  completionTime(questId: number): SessionTime | "initial" | undefined {
    if (this.initialCompleted.has(questId)) return "initial";
    return this.toggles.get(questId)?.find((toggle) => toggle.completed)?.t;
  }

  /** Quest ids that became completed during the session, with their first completion time. */
  completionsDuringSession(): TimedValue<number>[] {
    const result: TimedValue<number>[] = [];
    for (const [questId, list] of this.toggles) {
      if (this.initialCompleted.has(questId)) continue;
      const first = list.find((toggle) => toggle.completed);
      if (first) result.push({ t: first.t, v: questId });
    }
    return result.sort((a, b) => a.t - b.t);
  }

  logAt(t: SessionTime): ReadonlySet<number> {
    const index = lastIndexAtOrBefore(this.logTimes, t);
    return index === -1 ? new Set() : this.logSets[index];
  }

  isInLogAt(questId: number, t: SessionTime): boolean {
    return this.logAt(t).has(questId);
  }

  /** Level at `t`; before the first change point this is the first recorded level (the level at capture start). */
  levelAt(t: SessionTime): number | undefined {
    const index = lastIndexAtOrBefore(this.levelTimes, t);
    if (index === -1) return this.levelValues[0];
    return this.levelValues[index];
  }
}
