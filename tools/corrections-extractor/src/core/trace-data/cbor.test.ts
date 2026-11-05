import { describe, expect, it } from "vitest";
import { decodeCbor } from "./cbor";

const cbor = (...bytes: number[]) => Buffer.from(bytes);
const ascii = (text: string) => [...Buffer.from(text, "utf8")];

describe("decodeCbor", () => {
  it("should decode integers as doubles, like the Lua reference's byte-by-byte accumulation", () => {
    expect(decodeCbor(cbor(0x38, 0x63))).toBe(-100);
    // 2^53 + 1 cannot be represented; Lua and this decoder both round to 2^53.
    expect(decodeCbor(cbor(0x1b, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01))).toBe(2 ** 53);
  });

  it("should decode half, single and double floats", () => {
    expect(decodeCbor(cbor(0xf9, 0xc4, 0x00))).toBe(-4);
    expect(decodeCbor(cbor(0xf9, 0x00, 0x01))).toBe(2 ** -24);
    expect(decodeCbor(cbor(0xf9, 0x7c, 0x00))).toBe(Infinity);
    expect(decodeCbor(cbor(0xfa, 0x47, 0xc3, 0x50, 0x00))).toBe(100000);
    expect(decodeCbor(cbor(0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a))).toBe(1.1);
  });

  it("should turn arrays into JS arrays, and arrays with holes into objects keyed from 1", () => {
    expect(decodeCbor(cbor(0x82, 0x01, 0x02))).toEqual([1, 2]);
    expect(decodeCbor(cbor(0x83, 0x01, 0xf6, 0x03))).toEqual({ "1": 1, "3": 3 });
    expect(decodeCbor(cbor(0x83, 0x01, 0x02, 0xf6))).toEqual([1, 2]);
    expect(decodeCbor(cbor(0x80))).toEqual([]);
    expect(decodeCbor(cbor(0xa0))).toEqual([]);
  });

  it("should turn maps keyed 1..n into arrays", () => {
    expect(decodeCbor(cbor(0xa2, 0x02, 0x61, ...ascii("b"), 0x01, 0x61, ...ascii("a")))).toEqual(["a", "b"]);
    expect(decodeCbor(cbor(0xa1, 0x02, 0x61, ...ascii("b")))).toEqual({ "2": "b" });
    expect(decodeCbor(cbor(0xa2, 0x61, ...ascii("1"), 0x0a, 0x61, ...ascii("2"), 0x14))).toEqual([10, 20]);
    expect(decodeCbor(cbor(0xa3, 0x61, ...ascii("a"), 0x01, 0x61, ...ascii("a"), 0xf6, 0x01, 0x05))).toEqual([5]);
  });

  it("should keep packed args and tables with string keys as objects", () => {
    expect(decodeCbor(cbor(0xa2, 0x01, 0x07, 0x61, ...ascii("n"), 0x01))).toEqual({ "1": 7, n: 1 });
    expect(decodeCbor(cbor(0xa1, 0x61, ...ascii("x"), 0x82, 0x01, 0x02))).toEqual({ x: [1, 2] });
  });

  it("should keep tables whose first value is an integer-keyed table as objects, like index-keyed streams", () => {
    expect(decodeCbor(cbor(0x82, 0x81, 0x01, 0x81, 0x02))).toEqual({ "1": [1], "2": [2] });
    expect(decodeCbor(cbor(0x82, 0xa1, 0x02, 0x01, 0x81, 0x02))).toEqual({ "1": { "2": 1 }, "2": [2] });
    expect(decodeCbor(cbor(0x82, 0x80, 0x81, 0x02))).toEqual([[], [2]]);
    expect(decodeCbor(cbor(0x82, 0xa1, 0x61, ...ascii("x"), 0x01, 0x81, 0x02))).toEqual([{ x: 1 }, [2]]);
  });

  it("should keep only number and string keys and let a nil value remove an earlier key", () => {
    expect(decodeCbor(cbor(0xa1, 0x01, 0x61, ...ascii("x")))).toEqual(["x"]);
    expect(decodeCbor(cbor(0xa2, 0x61, ...ascii("a"), 0x01, 0x61, ...ascii("a"), 0xf6))).toEqual([]);
    expect(decodeCbor(cbor(0xa2, 0xf5, 0x01, 0x80, 0x02))).toEqual([]);
  });

  it("should keep non-canonical numeric string keys as object keys", () => {
    expect(decodeCbor(cbor(0xa1, 0x62, ...ascii("01"), 0x05))).toEqual({ "01": 5 });
    expect(decodeCbor(cbor(0xa1, 0x63, ...ascii("1.0"), 0x05))).toEqual({ "1.0": 5 });
  });

  it("should keep a __proto__ key as an own entry", () => {
    const table = decodeCbor(cbor(0xa1, 0x69, ...ascii("__proto__"), 0x01)) as Record<string, unknown>;
    expect(Object.getPrototypeOf(table)).toBe(Object.prototype);
    expect(Object.keys(table)).toEqual(["__proto__"]);
  });

  it("should read byte strings and text strings as UTF-8", () => {
    expect(decodeCbor(cbor(0x42, 0xc3, 0xa9))).toBe("é");
    expect(decodeCbor(cbor(0x62, 0xc3, 0xa9))).toBe("é");
  });

  it("should reject what BlizzardCBOR rejects", () => {
    expect(() => decodeCbor(cbor(0xc0, 0x00))).toThrow(/tags/);
    expect(() => decodeCbor(cbor(0x9f, 0xff))).toThrow(/indefinite/);
    expect(() => decodeCbor(cbor(0x01, 0x02))).toThrow(/trailing/);
    expect(() => decodeCbor(cbor(0x19, 0x01))).toThrow(/truncated/);
    expect(() => decodeCbor(cbor(0xa1, 0xf6, 0x01))).toThrow(/nil/);
  });

  it("should allow 100 nested tables and reject 101", () => {
    expect(() => decodeCbor(cbor(...Array(99).fill(0x81), 0x80))).not.toThrow();
    expect(() => decodeCbor(cbor(...Array(100).fill(0x81), 0x80))).toThrow(/depth/);
  });
});
