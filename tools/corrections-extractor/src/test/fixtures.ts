import type { EventEntry, FunctionStreamEntry, PackedArgs, SessionRecord } from "../core/types";

export function makeSession(functions: SessionRecord["functions"], events: SessionRecord["events"] = []): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events,
    functions,
    functionsDelta: {},
  };
}

export function creatureGuid(id: number): string {
  return `Creature-0-5208-0-7-${id}-000032`;
}

export function objectGuid(id: number): string {
  return `GameObject-0-5208-0-7-${id}-000032`;
}

/** An `undefined` value is recorded as nil, i.e. without a `v` key. */
export function apiSample(time: number, preciseTime: number, value?: unknown): FunctionStreamEntry {
  return value === undefined ? { t: time, tp: preciseTime } : { t: time, tp: preciseTime, v: value };
}

export function traceEvent(name: string, time: number, preciseTime: number, ...args: unknown[]): EventEntry {
  const packed: PackedArgs = { n: args.length };
  args.forEach((arg, index) => {
    packed[index + 1] = arg;
  });
  return { t: time, tp: preciseTime, e: name, a: packed };
}
