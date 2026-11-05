// CBOR decoder for the tables the addon exports (Blizzard's CBOR serializer), producing the
// shape every trace consumer works with. It keeps the behaviour of the former Lua toolchain
// (BlizzardCBOR, then lua-state and its normalization):
//
// - nil never appears; keys other than numbers and strings (booleans, tables) are dropped
// - a table whose keys are exactly 1..n becomes a JS array, unless its first value is itself
//   a table with only positive integer keys (index-keyed streams stay objects)
// - every other table becomes a plain object with string keys; packed args keep their `n`
//
// Supported subset, as in BlizzardCBOR: definite-length items only, no tags, simple
// values limited to false/true/null/undefined, half/single/double floats.

/** A decoded Lua value. `undefined` stands for Lua nil. */
export type LuaValue = undefined | boolean | number | string | LuaTable;
export type LuaTable = Exclude<LuaValue, undefined>[] | { [key: string]: Exclude<LuaValue, undefined> };
type RawTable = { [key: string]: Exclude<LuaValue, undefined> };

const MAX_NESTING_DEPTH = 100;

export class CborError extends Error {}

class Reader {
  private pos = 0;
  private readonly view: DataView;
  /**
   * Whether the table decoded last had only positive integer keys (and at least one). A table
   * whose first value is such a table stays an object.
   */
  private lastTableIntegerKeyed = false;

  constructor(private readonly bytes: Buffer) {
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  get done(): boolean {
    return this.pos >= this.bytes.length;
  }

  private need(count: number): void {
    if (this.pos + count > this.bytes.length) throw new CborError("truncated CBOR data");
  }

  /** Big-endian unsigned integer, accumulated in doubles exactly like the Lua reference. */
  private uint(byteCount: number): number {
    this.need(byteCount);
    let value = 0;
    for (let i = 0; i < byteCount; i++) value = value * 256 + this.bytes[this.pos + i];
    this.pos += byteCount;
    return value;
  }

  private argument(info: number): number {
    if (info < 24) return info;
    if (info === 24) return this.uint(1);
    if (info === 25) return this.uint(2);
    if (info === 26) return this.uint(4);
    if (info === 27) return this.uint(8);
    if (info === 31) throw new CborError("indefinite-length CBOR items are not supported");
    throw new CborError("invalid CBOR additional information");
  }

  value(depth: number): LuaValue {
    this.need(1);
    const initial = this.bytes[this.pos++];
    const major = initial >> 5;
    const info = initial & 31;

    switch (major) {
      case 0:
        return this.argument(info);
      case 1:
        return -1 - this.argument(info);
      case 2:
      case 3: {
        const length = this.argument(info);
        this.need(length);
        const text = this.text(this.pos, this.pos + length);
        this.pos += length;
        return text;
      }
      case 4:
        return this.array(info, depth);
      case 5:
        return this.map(info, depth);
      case 6:
        throw new CborError("CBOR tags are not supported");
      default:
        return this.simpleOrFloat(info);
    }
  }

  /** Byte strings are read as UTF-8, invalid bytes becoming U+FFFD. */
  private text(start: number, end: number): string {
    // Buffer#toString is slow for the many short keys and values; decode short ASCII in JS.
    if (end - start > 32) return this.bytes.toString("utf8", start, end);
    let text = "";
    for (let i = start; i < end; i++) {
      const byte = this.bytes[i];
      if (byte >= 0x80) return this.bytes.toString("utf8", start, end);
      text += String.fromCharCode(byte);
    }
    return text;
  }

  private array(info: number, depth: number): LuaTable {
    if (depth > MAX_NESTING_DEPTH) throw new CborError(`CBOR deserialization exceeded maximum table depth of ${MAX_NESTING_DEPTH}`);
    const length = this.argument(info);
    const items: Exclude<LuaValue, undefined>[] = [];
    let firstIntegerKeyed = false;
    let raw: RawTable | undefined;
    for (let index = 1; index <= length; index++) {
      const item = this.value(depth + 1);
      if (index === 1) firstIntegerKeyed = typeof item === "object" && this.lastTableIntegerKeyed;
      // Lua: items[index] = nil leaves a hole, so the table is no plain array any more.
      if (item === undefined) raw ??= Object.fromEntries(items.map((value, i) => [i + 1, value]));
      else if (raw) raw[index] = item;
      else items.push(item);
    }
    if (raw) return this.finish(raw, firstIntegerKeyed);
    this.lastTableIntegerKeyed = items.length > 0;
    return firstIntegerKeyed ? Object.fromEntries(items.map((value, i) => [i + 1, value])) : items;
  }

  private map(info: number, depth: number): LuaTable {
    if (depth > MAX_NESTING_DEPTH) throw new CborError(`CBOR deserialization exceeded maximum table depth of ${MAX_NESTING_DEPTH}`);
    const length = this.argument(info);
    const table: RawTable = {};
    let firstIntegerKeyed = false;
    let hasOtherKey = false;
    let hasDeleted = false;
    for (let i = 0; i < length; i++) {
      const key = this.value(depth + 1);
      if (key === undefined) throw new CborError("CBOR map keys that decode to nil are not supported");
      const item = this.value(depth + 1);
      const itemIntegerKeyed = typeof item === "object" && this.lastTableIntegerKeyed;
      // Only number and string keys are kept; boolean and table keys are skipped.
      if (typeof key !== "number" && typeof key !== "string") continue;
      if (typeof key === "number" && Number.isNaN(key)) throw new CborError("table index is NaN");
      const name = String(key);
      // Lua: map[key] = nil removes an earlier entry with the same key.
      if (item === undefined) {
        delete table[name];
        hasDeleted = true;
      } else {
        // defineProperty, so a "__proto__" key stays an own entry instead of changing the prototype.
        Object.defineProperty(table, name, { value: item, enumerable: true, writable: true, configurable: true });
        if (!hasOtherKey && !isIntegerKey(key)) hasOtherKey = true;
      }
      if (name === "1") firstIntegerKeyed = itemIntegerKeyed;
    }
    // Fast path for the common case: a key that is no positive integer makes it an object.
    if (hasOtherKey && !hasDeleted) {
      this.lastTableIntegerKeyed = false;
      return table;
    }
    return this.finish(table, firstIntegerKeyed);
  }

  /** Turns a table with string keys into an array or object, following the rules above. */
  private finish(raw: RawTable, firstIntegerKeyed: boolean): LuaTable {
    const keys = Object.keys(raw);
    const integerKeys = keys.map(Number);
    const integerKeyed = keys.every(isIntegerKey);
    this.lastTableIntegerKeyed = integerKeyed && keys.length > 0;
    if (keys.length === 0) return [];
    // Packed args stay objects through their `n` key, index-keyed streams through firstIntegerKeyed.
    if (!integerKeyed || firstIntegerKeyed) return raw;
    integerKeys.sort((a, b) => a - b);
    if (integerKeys.some((n, i) => n !== i + 1)) return raw;
    return integerKeys.map((n) => raw[String(n)]);
  }

  private simpleOrFloat(info: number): LuaValue {
    switch (info) {
      case 20:
        return false;
      case 21:
        return true;
      case 22:
      case 23:
        return undefined;
      case 24:
        throw new CborError(`unsupported CBOR simple value ${this.uint(1)}`);
      case 25:
        return decodeHalf(this.uint(2));
      case 26: {
        this.need(4);
        const value = this.view.getFloat32(this.pos);
        this.pos += 4;
        return value;
      }
      case 27: {
        this.need(8);
        const value = this.view.getFloat64(this.pos);
        this.pos += 8;
        return value;
      }
      case 31:
        throw new CborError("CBOR break values are not supported");
      default:
        throw new CborError(`unsupported CBOR simple value ${info}`);
    }
  }
}

/** Whether the string form of `key` reads back as a positive integer. */
function isIntegerKey(key: number | string): boolean {
  if (typeof key === "number") return Number.isInteger(key) && key >= 1;
  const first = key.charCodeAt(0);
  // Names starting with a letter or underscore never read back as numbers.
  if ((first >= 65 && first <= 90) || (first >= 97 && first <= 122) || first === 95) return false;
  // Only canonical forms: "01", "1.0" or " 1" stay object keys.
  const n = Number(key);
  return Number.isInteger(n) && n >= 1 && String(n) === key;
}

function decodeHalf(bits: number): number {
  const sign = bits & 0x8000 ? -1 : 1;
  const exponent = (bits >> 10) & 0x1f;
  const mantissa = bits & 0x3ff;
  if (exponent === 0) return mantissa === 0 ? sign * 0 : sign * mantissa * 2 ** -24;
  if (exponent === 31) return mantissa === 0 ? sign * Infinity : NaN;
  return sign * (1 + mantissa / 1024) * 2 ** (exponent - 15);
}

/** Decodes exactly one CBOR item; trailing bytes are an error, as in BlizzardCBOR.DeserializeCBOR. */
export function decodeCbor(bytes: Buffer): LuaValue {
  const reader = new Reader(bytes);
  const value = reader.value(1);
  if (!reader.done) throw new CborError("trailing data after CBOR item");
  return value;
}
