// CBOR decoder matching the Lua reference (tools/decoder/dependencies/BlizzardCBOR.lua)
// and the way lua-state hands Lua tables to JavaScript.
//
// The output is what `loadTraceFile` sees *before* `normalizeLuaValue`: every Lua table
// becomes a plain object with string keys, nil never appears, and keys lua-state cannot
// represent (booleans, tables) are dropped. Feeding it through `normalizeLuaValue` then
// yields exactly the shape every trace consumer already works with.
//
// Supported subset, as in BlizzardCBOR: definite-length items only, no tags, simple
// values limited to false/true/null/undefined, half/single/double floats.

/** A decoded Lua value. `undefined` stands for Lua nil. */
export type LuaValue = undefined | boolean | number | string | LuaTable;
export interface LuaTable {
  [key: string]: Exclude<LuaValue, undefined>;
}

const MAX_NESTING_DEPTH = 100;

export class CborError extends Error {}

class Reader {
  private pos = 0;
  private readonly view: DataView;

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
        // lua-state builds JS strings from Lua byte strings as UTF-8 (invalid bytes become U+FFFD).
        const text = this.bytes.toString("utf8", this.pos, this.pos + length);
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

  private array(info: number, depth: number): LuaTable {
    if (depth > MAX_NESTING_DEPTH) throw new CborError(`CBOR deserialization exceeded maximum table depth of ${MAX_NESTING_DEPTH}`);
    const length = this.argument(info);
    const table: LuaTable = {};
    for (let index = 1; index <= length; index++) {
      const item = this.value(depth + 1);
      // Lua: items[index] = nil leaves a hole.
      if (item !== undefined) table[index] = item;
    }
    return table;
  }

  private map(info: number, depth: number): LuaTable {
    if (depth > MAX_NESTING_DEPTH) throw new CborError(`CBOR deserialization exceeded maximum table depth of ${MAX_NESTING_DEPTH}`);
    const length = this.argument(info);
    const table: LuaTable = {};
    for (let i = 0; i < length; i++) {
      const key = this.value(depth + 1);
      if (key === undefined) throw new CborError("CBOR map keys that decode to nil are not supported");
      const item = this.value(depth + 1);
      // lua-state only converts number and string keys; boolean and table keys are skipped.
      if (typeof key !== "number" && typeof key !== "string") continue;
      if (typeof key === "number" && Number.isNaN(key)) throw new CborError("table index is NaN");
      const name = String(key);
      // Lua: map[key] = nil removes an earlier entry with the same key.
      if (item === undefined) delete table[name];
      else table[name] = item;
    }
    return table;
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
