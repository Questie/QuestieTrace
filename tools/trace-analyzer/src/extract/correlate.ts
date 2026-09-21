// Shared time-window correlation: "what value of this stream was closest to time t?"
//
// This is the crux of positional approximation (e.g. NPC/object spawn coords ~=
// player coords at interaction time) and any other "nearest observation" lookup
// that isn't simply "latest value at or before t" (which `core/emulator.ts`'s
// `valueAt` already covers).

import type { FunctionStreamEntry } from "../core/types";

/** Single global correlation window (decision: tune later only if needed). */
export const DEFAULT_CORRELATION_WINDOW_SECONDS = 5;

/**
 * Returns the `v` of the stream entry whose `t` is closest to `targetT`, as long
 * as it's within `windowSeconds`. Considers entries both before and after
 * `targetT` (unlike `valueAt`, which only looks backward).
 *
 * Returns `null` if the stream is empty/undefined or nothing falls within the
 * window.
 */
export function nearestByTime<T = unknown>(
  stream: FunctionStreamEntry[] | undefined,
  targetT: number,
  windowSeconds: number = DEFAULT_CORRELATION_WINDOW_SECONDS,
): T | null {
  if (!stream || stream.length === 0) {
    return null;
  }

  let lo = 0;
  let hi = stream.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (stream[mid].t < targetT) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }

  let best: FunctionStreamEntry | null = null;
  let bestDiff = Infinity;
  for (const idx of [lo - 1, lo]) {
    const entry = stream[idx];
    if (!entry) continue;
    const diff = Math.abs(entry.t - targetT);
    if (diff <= windowSeconds && diff < bestDiff) {
      best = entry;
      bestDiff = diff;
    }
  }

  if (!best) {
    return null;
  }
  return (best.v as T) ?? null;
}
