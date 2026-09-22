// ============================================================
// TypeScript types matching SCHEMA_SPEC v9
// ============================================================

/**
 * Packed args: sparse object with authoritative `n` field.
 * Indices 1..n hold values (some may be null/undefined for Lua nil).
 * The `n` field gives the true argument count.
 */
export interface PackedArgs {
  [index: number]: unknown;
  n: number;
}

/** A single entry in a function stream */
export interface FunctionStreamEntry {
  t: number;
  tp: number;
  /** absent means nil (Lua serialization omits nil keys) */
  v?: unknown;
}

/** A single event in the events timeline */
export interface EventEntry {
  t: number;
  tp: number;
  e: string;
  a: PackedArgs;
}

/** A single delta entry (add/remove from a set) */
export interface DeltaEntry {
  t: number;
  tp: number;
  add?: number[];
  remove?: number[];
}

/** Delta stream: initial set + ordered delta entries */
export interface DeltaStream {
  t: number;
  tp: number;
  initial: number[];
  delta: DeltaEntry[];
}

/**
 * A function stream is either:
 * - Parameterless: a flat array of FunctionStreamEntry
 * - Parameterized: a record mapping param key → FunctionStream
 *
 * Multi-argument APIs use nested parameterized records in native argument order,
 * ending at a FunctionStreamEntry[] leaf. One-level streams remain the common
 * case; the recursive shape exists so two-argument APIs such as
 * `GetQuestLogRewardInfo(rewardIndex, questId)` can be represented without
 * composite string keys.
 */
export interface FunctionStreamMap {
  /** Lua table keys are normalized to strings by the loader. */
  [key: string]: FunctionStream;
}

export type FunctionStream = FunctionStreamEntry[] | FunctionStreamMap;

/**
 * A complete session recording.
 *
 * `name`/`stoppedAt`/`stoppedAtPrecise`/`duration`/`durationPrecise` are only
 * set once a session has been explicitly stopped/saved (see
 * QuestieTrace.lua). Exports also bundle the in-progress `currentSession`
 * (if any), which has none of those fields yet - callers must not assume
 * they're present.
 */
export interface SessionRecord {
  schemaVersion: number;
  /** 1 = observed raw API streams; absent = legacy/unknown recording contract. */
  recordingContractVersion?: number;
  name?: string;
  startedAt: number;
  startedAtPrecise: number;
  stoppedAt?: number;
  stoppedAtPrecise?: number;
  duration?: number;
  durationPrecise?: number;
  events: EventEntry[];
  functions: Record<string, FunctionStream>;
  functionsDelta: Record<string, DeltaStream>;
}

/** Top-level per-character SavedVariables */
export interface TraceFile {
  lastSavedSession?: string;
  sessions: SessionRecord[];
}

/** Summary of a trace file in the Traces directory */
export interface TraceFileSummary {
  name: string;
  sessionCount: number | null;
  error: string | null;
}

/**
 * Summary sent to browser for session list.
 *
 * `index` is the session's position in the file's `sessions` array and is
 * the stable identifier used to address it (`name` may be absent - see
 * `SessionRecord`).
 */
export interface SessionSummary {
  index: number;
  name: string | null;
  duration: number | null;
  startedAt: number;
  eventCount: number;
  functionCount: number;
}

/** Event category for filtering */
export type EventCategory =
  | "quest"
  | "loot"
  | "combat"
  | "chat"
  | "target"
  | "zone"
  | "inventory"
  | "npc"
  | "init"
  | "other";
