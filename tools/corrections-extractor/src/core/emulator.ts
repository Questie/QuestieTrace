// ============================================================
// Emulation engine — port of FUNCTION_EMULATION_SPEC v9
//
// Reconstructs WoW API function outputs at a target time `t`
// from QuestieTrace session data.
// ============================================================

import type {
  SessionRecord,
  FunctionStream,
  FunctionStreamEntry,
  FunctionStreamMap,
} from "./types.js";

/**
 * Get the list of parameter keys for a parameterized stream node.
 * Returns empty array for leaf stream arrays.
 */
export function getParamKeys(stream: FunctionStream): string[] {
  if (Array.isArray(stream)) return [];
  return Object.keys(stream);
}

/**
 * Get the leaf entry array for a function stream.
 *
 * Parameters are walked in native API argument order. This preserves existing
 * one-level lookups while also supporting nested streams such as
 * `functions["GetQuestLogRewardInfo"][rewardIndex][questId]`.
 */
export function getStream(
  session: SessionRecord,
  name: string,
  ...params: Array<string | number>
): FunctionStreamEntry[] | undefined {
  let node: FunctionStream | undefined = session.functions[name];
  if (!node) return undefined;

  for (const param of params) {
    if (Array.isArray(node)) return undefined;
    const paramMap = node as FunctionStreamMap;
    // Lua table keys are normalized to strings by the loader.
    node = paramMap[String(param)];
    if (!node) return undefined;
  }

  // A parameterized map without all required params is not a readable stream.
  return Array.isArray(node) ? node : undefined;
}

/**
 * Binary search for the value at a target time.
 *
 * Returns the `v` field of the latest entry with `t <= targetT`.
 * Returns `undefined` if no entry exists at or before targetT.
 */
export function valueAt(
  stream: FunctionStreamEntry[],
  targetT: number
): unknown {
  if (stream.length === 0) return undefined;

  let lo = 0;
  let hi = stream.length - 1;
  let result = -1;

  while (lo <= hi) {
    const mid = (lo + hi) >>> 1;
    if (stream[mid].t <= targetT) {
      result = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }

  if (result === -1) return undefined;
  return stream[result].v;
}

/**
 * Unpack a stored value for display.
 *
 * - Packed args (object with `n`): extract indices 1..n into array
 * - Other objects/scalars: return as-is
 */
export function emulate(v: unknown): unknown {
  if (v !== null && v !== undefined && typeof v === "object" && !Array.isArray(v)) {
    const obj = v as Record<string, unknown>;
    if (typeof obj.n === "number") {
      const result: unknown[] = [];
      for (let i = 1; i <= (obj.n as number); i++) {
        result.push(obj[i] ?? null);
      }
      return result;
    }
  }
  return v;
}
