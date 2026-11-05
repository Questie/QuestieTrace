// Decodes trace-data export strings natively, reversing Modules/Export/Encoding.lua:
//
//   "!QuestieTrace:N!" + EncodeForPrint(raw deflate(CBOR(payload))) + "!End:QuestieTrace:N!"

import { inflateRawSync } from "zlib";
import type { TraceFile } from "../types";
import { decodeCbor } from "./cbor";

const PRINT_ALPHABET = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789()";
const SIX_BIT = new Int8Array(256).fill(-1);
for (let i = 0; i < PRINT_ALPHABET.length; i++) SIX_BIT[PRINT_ALPHABET.charCodeAt(i)] = i;

// The print alphabet has no "!", so a payload never spans another marker: an export pasted into
// the middle of another one is matched on its own, and the one it cut off has no end marker.
const START_PATTERN = /!QuestieTrace:\d+!/g;
const EXPORT_PATTERN = /!QuestieTrace:(\d+)!([^!]*)!End:QuestieTrace:\1!/g;
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

/** Decodes one print-encoded payload (the text between the markers) into its normalized table shape. */
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

export interface DecodedExportString {
  /** One per export that decoded. */
  traces: TraceFile[];
  /** Why each other export was dropped. */
  dropped: string[];
}

/**
 * Decodes every export in a submission's `export_string`. Some submissions hold several
 * exports, pasted back to back or one into the middle of another; each becomes its own
 * TraceFile. Exports that are cut off or don't decode are dropped; it throws only when none
 * decodes.
 */
export function decodeExportString(exportString: string): DecodedExportString {
  const starts = [...exportString.matchAll(START_PATTERN)].map((match) => match.index);
  if (starts.length === 0) throw new Error("export string must be wrapped in !QuestieTrace:N! and !End:QuestieTrace:N! markers");
  const complete = new Map([...exportString.matchAll(EXPORT_PATTERN)].map((match) => [match.index, match[2]]));

  const traces: TraceFile[] = [];
  const dropped: string[] = [];
  starts.forEach((start, index) => {
    const payload = complete.get(start);
    if (payload === undefined) {
      dropped.push(`export ${index + 1} has no matching end marker`);
    } else if (payload.trim() === "") {
      dropped.push(`export ${index + 1}: export markers found but payload is empty`);
    } else {
      try {
        traces.push(decodePayload(payload) as TraceFile);
      } catch (e) {
        dropped.push(`export ${index + 1}: ${e instanceof Error ? e.message : String(e)}`);
      }
    }
  });
  if (traces.length === 0) throw new Error(dropped.join("; "));
  return { traces, dropped };
}
