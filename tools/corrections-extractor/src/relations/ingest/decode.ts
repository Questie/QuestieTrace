// Decodes trace-data export strings natively, reversing Modules/Export/Encoding.lua:
//
//   "!QuestieTrace:N!" + EncodeForPrint(raw deflate(CBOR(payload))) + "!End:QuestieTrace:N!"
//
// The result has exactly the shape `loadTraceFile` (lua-state + normalizeLuaValue) gives
// for the same data, so the extractor's stream helpers work on it unchanged. The Lua
// toolchain in tools/decoder is the reference; ingest/decode.oracle.test.ts compares both.

import { inflateRawSync } from "zlib";
import { normalizeLuaValue } from "../../core/normalize";
import type { TraceFile } from "../../core/types";
import { decodeCbor } from "./cbor";

const PRINT_ALPHABET = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789()";
const SIX_BIT = new Int8Array(256).fill(-1);
for (let i = 0; i < PRINT_ALPHABET.length; i++) SIX_BIT[PRINT_ALPHABET.charCodeAt(i)] = i;

const EXPORT_PATTERN = /!QuestieTrace:(\d+)!(.*?)!End:QuestieTrace:\1!/gs;
/** Far above any real export (the largest inflate to a few dozen MB); stops a hostile payload from exhausting memory. */
const MAX_CBOR_BYTES = 512 * 1024 * 1024;

/**
 * Reverses LibDeflate:EncodeForPrint: 4 characters carry 3 bytes, least significant 6 bits
 * first; a short tail carries the remaining whole bytes. Whitespace is ignored anywhere, so
 * exports that were line-wrapped while being copied still decode (the Lua reference only
 * trims the ends).
 */
export function decodeForPrint(text: string): Buffer {
  const clean = text.replace(/\s+/g, "");
  if (clean.length === 1) throw new Error("payload is not valid print-encoded data");
  const out = Buffer.allocUnsafe(Math.floor((clean.length * 6) / 8));
  let size = 0;
  let cache = 0;
  let bits = 0;
  for (let i = 0; i < clean.length; i++) {
    const code = clean.charCodeAt(i);
    const value = code < 256 ? SIX_BIT[code] : -1;
    if (value < 0) throw new Error(`payload is not valid print-encoded data (character ${JSON.stringify(clean[i])})`);
    cache |= value << bits;
    bits += 6;
    if (bits >= 8) {
      out[size++] = cache & 0xff;
      cache >>>= 8;
      bits -= 8;
    }
  }
  return out.subarray(0, size);
}

/** Decodes one print-encoded payload (the text between the markers) into its raw Lua table shape. */
export function decodePayload(encoded: string): unknown {
  const compressed = decodeForPrint(encoded);
  let cbor: Buffer;
  try {
    cbor = inflateRawSync(compressed, { maxOutputLength: MAX_CBOR_BYTES });
  } catch (e) {
    throw new Error(`payload could not be decompressed: ${e instanceof Error ? e.message : String(e)}`);
  }
  const value = decodeCbor(cbor);
  if (value === null || typeof value !== "object") throw new Error(`decoded payload is not a table (got ${typeof value})`);
  return value;
}

/** The individual "!QuestieTrace:N!...!End:QuestieTrace:N!" exports in a submission (usually one). */
export function splitExports(exportString: string): string[] {
  return [...exportString.matchAll(EXPORT_PATTERN)].map((match) => match[2]);
}

/**
 * Decodes every export in a submission's `export_string`. Some submissions hold several
 * exports pasted back to back; each becomes its own TraceFile.
 */
export function decodeExportString(exportString: string): TraceFile[] {
  const payloads = splitExports(exportString);
  if (payloads.length === 0) throw new Error("export string must be wrapped in !QuestieTrace:N! and !End:QuestieTrace:N! markers");
  return payloads.map((payload) => {
    if (payload.trim() === "") throw new Error("export markers found but payload is empty");
    return normalizeLuaValue(decodePayload(payload)) as TraceFile;
  });
}
