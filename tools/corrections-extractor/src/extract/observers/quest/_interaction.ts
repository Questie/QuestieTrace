import { getStream } from "../../../core/emulator";
import type { FunctionStreamEntry, SessionRecord } from "../../../core/types";

/** An uncertain clock boundary must not be treated as a nil value that permits giver fallback. */
export const AMBIGUOUS_INTERACTION = Symbol("ambiguous interaction");

/** Reads one API stream within a quest interaction's time boundary; may return AMBIGUOUS_INTERACTION. */
export type InteractionLookup = (name: string, ...params: Array<string | number>) => unknown;

/**
 * Includes the event's synchronous probes, which are recorded after its raw
 * timestamp, but excludes probes from the next event in the same frame.
 * Indistinguishable event timestamps cannot safely identify a quest giver.
 */
export function questEventLookup(session: SessionRecord, index: number): InteractionLookup | undefined {
  const event = session.events[index];
  const previous = session.events[index - 1];
  const next = session.events[index + 1];
  if ((previous?.t === event.t && previous.tp === event.tp)
    || (next?.t === event.t && next.tp === event.tp)) return undefined;

  const beforeTp = next?.t === event.t ? next.tp : Infinity;
  return (name, ...params) => valueBeforeNextEvent(getStream(session, name, ...params) ?? [], event.t, beforeTp);
}

function valueBeforeNextEvent(stream: FunctionStreamEntry[], t: number, tp: number): unknown {
  let lo = 0;
  let hi = stream.length - 1;
  let result = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >>> 1;
    const sample = stream[mid];
    const within = sample.t < t || (sample.t === t && sample.tp <= tp);
    if (within) {
      result = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  if (result < 0) return undefined;
  const selected = stream[result];
  if (selected.t === t && selected.tp === tp) {
    // A probe tied with the next raw event might belong to either interaction.
    // Excluding it and returning the preceding GUID could resurrect stale state.
    return AMBIGUOUS_INTERACTION;
  }
  // Preserve recorded nil transitions; never search back for an older non-nil GUID.
  return selected.v;
}
