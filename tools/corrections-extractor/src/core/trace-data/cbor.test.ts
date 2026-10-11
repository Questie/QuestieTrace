import { describe, expect, it } from "vitest";
import { decodeCbor } from "./cbor";

const cbor = (...bytes: number[]) => Buffer.from(bytes);
const ascii = (text: string) => [...Buffer.from(text, "utf8")];

describe("decodeCbor (BlizzardCBOR semantics as seen through lua-state)", () => {
  it("decodes integers as doubles, like the Lua reference's byte-by-byte accumulation", () => {
    expect(decodeCbor(cbor(0x38, 0x63))).toBe(-100);
    // 2^53 + 1 cannot be represented; Lua and this decoder both round to 2^53.
    expect(decodeCbor(cbor(0x1b, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01))).toBe(2 ** 53);
  });

  it("decodes half, single and double floats", () => {
    expect(decodeCbor(cbor(0xf9, 0xc4, 0x00))).toBe(-4);
    expect(decodeCbor(cbor(0xf9, 0x00, 0x01))).toBe(2 ** -24);
    expect(decodeCbor(cbor(0xf9, 0x7c, 0x00))).toBe(Infinity);
    expect(decodeCbor(cbor(0xfa, 0x47, 0xc3, 0x50, 0x00))).toBe(100000);
    expect(decodeCbor(cbor(0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a))).toBe(1.1);
  });

  it("turns arrays into Lua tables keyed from 1, leaving holes for null", () => {
    expect(decodeCbor(cbor(0x83, 0x01, 0xf6, 0x03))).toEqual({ "1": 1, "3": 3 });
    expect(decodeCbor(cbor(0x80))).toEqual({});
  });

  it("keeps only the keys lua-state can represent and lets a nil value remove an earlier key", () => {
    expect(decodeCbor(cbor(0xa1, 0x01, 0x61, ...ascii("x")))).toEqual({ "1": "x" });
    expect(decodeCbor(cbor(0xa2, 0x61, ...ascii("a"), 0x01, 0x61, ...ascii("a"), 0xf6))).toEqual({});
    expect(decodeCbor(cbor(0xa2, 0xf5, 0x01, 0x80, 0x02))).toEqual({});
  });

  it("reads byte strings and text strings as UTF-8", () => {
    expect(decodeCbor(cbor(0x42, 0xc3, 0xa9))).toBe("é");
    expect(decodeCbor(cbor(0x62, 0xc3, 0xa9))).toBe("é");
  });

  it("rejects what BlizzardCBOR rejects", () => {
    expect(() => decodeCbor(cbor(0xc0, 0x00))).toThrow(/tags/);
    expect(() => decodeCbor(cbor(0x9f, 0xff))).toThrow(/indefinite/);
    expect(() => decodeCbor(cbor(0x01, 0x02))).toThrow(/trailing/);
    expect(() => decodeCbor(cbor(0x19, 0x01))).toThrow(/truncated/);
    expect(() => decodeCbor(cbor(0xa1, 0xf6, 0x01))).toThrow(/nil/);
  });

  it("allows 100 nested tables and rejects 101", () => {
    expect(() => decodeCbor(cbor(...Array(99).fill(0x81), 0x80))).not.toThrow();
    expect(() => decodeCbor(cbor(...Array(100).fill(0x81), 0x80))).toThrow(/depth/);
  });
});
