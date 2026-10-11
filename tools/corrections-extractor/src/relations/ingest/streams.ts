// Point-in-time reads over raw session streams, ordered by `tp` (GetTimePreciseSec).
//
// Why not `t`: GetTime() is cached per frame, so a QUEST_DETAIL, the QUEST_FINISHED that
// closes it and both of their samples can all share one `t`. Looking a value up "at t" then
// returns the *last* sample of the frame (often the closed state: GetQuestID = 0). `tp` is
// unique per call and the core records an event before dispatching it to trackers, so the
// samples taken for event i lie between events[i].tp and events[i + 1].tp.

import type { EventEntry, FunctionStream, FunctionStreamEntry, SessionRecord } from "../../core/types";

export function entryTime(entry: { t: number; tp?: number }): number {
  return typeof entry.tp === "number" ? entry.tp : entry.t;
}

/** The leaf stream at `functions[name][...params]`, or undefined when absent or not a leaf. */
export function leafStream(session: SessionRecord, name: string, ...params: Array<string | number>): FunctionStreamEntry[] | undefined {
  let node: FunctionStream | undefined = session.functions?.[name];
  for (const param of params) {
    if (!node || Array.isArray(node)) return undefined;
    node = node[String(param)];
  }
  return Array.isArray(node) ? node : undefined;
}

/**
 * Index of the last entry with time < `before` (or <= with `inclusive`), or -1.
 * Streams are appended in time order.
 */
export function lastIndexBefore(stream: readonly FunctionStreamEntry[], before: number, inclusive = false): number {
  let lo = 0;
  let hi = stream.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const time = entryTime(stream[mid]);
    if (time < before || (inclusive && time === before)) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

/** The value in effect just before `before` (Lua nil and "never sampled" are both undefined). */
export function valueBefore(stream: readonly FunctionStreamEntry[] | undefined, before: number): unknown {
  if (!stream) return undefined;
  const index = lastIndexBefore(stream, before);
  return index === -1 ? undefined : stream[index].v;
}

/** The value of the sample taken at `time` or the latest one before it. */
export function valueAtOrBefore(stream: readonly FunctionStreamEntry[] | undefined, time: number): unknown {
  if (!stream) return undefined;
  const index = lastIndexBefore(stream, time, true);
  return index === -1 ? undefined : stream[index].v;
}

/** True when the stream has any sample before `before` (a read happened, even if it returned nil). */
export function sampledBefore(stream: readonly FunctionStreamEntry[] | undefined, before: number): boolean {
  return !!stream && lastIndexBefore(stream, before) !== -1;
}

/** Entries with from <= time < to. */
export function entriesBetween(stream: readonly FunctionStreamEntry[] | undefined, from: number, to: number): FunctionStreamEntry[] {
  if (!stream) return [];
  const end = lastIndexBefore(stream, to);
  const result: FunctionStreamEntry[] = [];
  for (let i = end; i >= 0 && entryTime(stream[i]) >= from; i--) result.push(stream[i]);
  return result.reverse();
}

/** Packed args / packed returns: `{1: a, 2: b, n: 2}` -> value at 1-based `index`. */
export function packed(value: unknown, index: number): unknown {
  if (!value || typeof value !== "object") return undefined;
  if (Array.isArray(value)) return value[index - 1];
  return (value as Record<string, unknown>)[String(index)];
}

export function eventArg(event: EventEntry, index: number): unknown {
  return packed(event.a, index);
}

export function asPositiveInt(value: unknown): number | undefined {
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : undefined;
}
