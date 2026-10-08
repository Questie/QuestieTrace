// ============================================================
// Normalize lua-state output to proper JS types
//
// lua-state converts ALL Lua tables to plain JS objects with
// string keys. We must detect which are arrays vs objects vs
// packed args and convert accordingly.
// ============================================================

/**
 * Check if a plain object from lua-state represents a Lua array.
 *
 * A Lua array has consecutive integer keys starting at 1 with no
 * non-integer keys. Objects with an `n` key are packed args (NOT
 * simple arrays) and should be kept as objects.
 */
function isLuaArray(obj: Record<string, unknown>): boolean {
  const keys = Object.keys(obj);
  if (keys.length === 0) return true; // empty table → empty array

  // If it has an "n" key with a number value, it's packed args
  if ("n" in obj && typeof obj["n"] === "number") return false;

  // Check if all keys are sequential integers starting at 1
  const numericKeys: number[] = [];
  for (const k of keys) {
    const n = Number(k);
    if (!Number.isInteger(n) || n < 1) return false; // has a non-integer key
    numericKeys.push(n);
  }

  numericKeys.sort((a, b) => a - b);
  // Must start at 1 and be consecutive
  for (let i = 0; i < numericKeys.length; i++) {
    if (numericKeys[i] !== i + 1) return false;
  }

  // Disambiguate flat arrays from parameterized streams keyed by slot/index.
  // Both have consecutive integer keys after lua-state conversion.
  // In a parameterized stream, each value is itself an entry array (object
  // with numeric keys). In a flat array, values are entries or scalars.
  // Check the first value: if it's an object whose keys are also all numeric
  // (i.e. a nested array-like table), this is a parameterized stream.
  const firstValue = obj["1"];
  if (
    firstValue !== null &&
    firstValue !== undefined &&
    typeof firstValue === "object" &&
    !Array.isArray(firstValue)
  ) {
    const childKeys = Object.keys(firstValue as Record<string, unknown>);
    const allChildNumeric =
      childKeys.length > 0 &&
      childKeys.every((k) => {
        const n = Number(k);
        return Number.isInteger(n) && n >= 1;
      });
    if (allChildNumeric) return false; // parameterized stream, not flat array
  }

  return true;
}

/**
 * Recursively normalize a value from lua-state output.
 *
 * - Lua arrays (consecutive integer keys, no `n`) → JS arrays
 * - Packed args (has `n` field) → keep as object, normalize children
 * - Named-key objects → keep as objects, normalize children
 * - Scalars/null → pass through
 */
export function normalizeLuaValue(value: unknown): unknown {
  if (value === null || value === undefined) return value;
  if (typeof value !== "object") return value;

  // Already a JS array (shouldn't happen from lua-state, but be safe)
  if (Array.isArray(value)) {
    return value.map(normalizeLuaValue);
  }

  const obj = value as Record<string, unknown>;

  // Check if this is a Lua array (sequential 1-based integer keys, no `n`)
  if (isLuaArray(obj)) {
    const keys = Object.keys(obj);
    if (keys.length === 0) return [];

    const arr: unknown[] = [];
    for (let i = 1; i <= keys.length; i++) {
      arr.push(normalizeLuaValue(obj[String(i)]));
    }
    return arr;
  }

  // Not an array — normalize children recursively
  const result: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(obj)) {
    result[k] = normalizeLuaValue(v);
  }
  return result;
}
