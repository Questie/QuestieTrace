// Opt-in check of the native decoder against the Lua reference toolchain on real submissions:
//
//   RELATIONS_ORACLE=1 npx vitest run src/relations/ingest/decode.oracle.test.ts
//
// Needs a trace-data checkout (TRACE_DATA_DIR) and lua5.1 with luafilesystem. Checks the first,
// middle and last submission by default; RELATIONS_ORACLE_FILES=a.json,b.json picks others.

import { execFileSync } from "child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import { describe, expect, it } from "vitest";
import { loadTraceFile } from "../../core/loader";
import { paths } from "../core/paths";
import { decodeExportString, splitExports } from "./decode";

const enabled = process.env.RELATIONS_ORACLE === "1";
const decoderDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../decoder");

function oracleFiles(): string[] {
  if (process.env.RELATIONS_ORACLE_FILES) return process.env.RELATIONS_ORACLE_FILES.split(",");
  const root = resolve(paths.traceDataDir, "submissions");
  const files = readdirSync(root)
    .flatMap((month) => readdirSync(resolve(root, month)).map((name) => resolve(root, month, name)))
    .sort();
  return [files[0], files[files.length >> 1], files[files.length - 1]];
}

/** decoder.lua serializes numbers with Lua 5.1 tostring(), which keeps 14 significant digits. */
function round14(value: unknown): unknown {
  if (typeof value === "number") return Number.isFinite(value) ? Number(value.toPrecision(14)) : value;
  if (Array.isArray(value)) return value.map(round14);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, round14(item)]));
  return value;
}

/** Decodes one payload with tools/decoder/decoder.lua and loads it the way Traces/ files are loaded. */
function luaDecode(payload: string, scratch: string): unknown {
  const input = resolve(scratch, "export.txt");
  const output = resolve(scratch, "export.lua");
  writeFileSync(input, `!QuestieTrace:1!${payload}!End:QuestieTrace:1!`);
  execFileSync("lua5.1", ["decoder.lua", input, "-o", output], { cwd: decoderDir });
  writeFileSync(output, readFileSync(output, "utf8").replace(/^return \{/, "QuestieTraceCharacter = {"));
  return loadTraceFile(output);
}

describe.skipIf(!enabled)("decodeExportString matches the Lua decoder", () => {
  it.each(enabled ? oracleFiles() : [])("%s", (file) => {
    const exportString = (JSON.parse(readFileSync(file, "utf8")) as { export_string: string }).export_string;
    const decoded = decodeExportString(exportString);
    const scratch = mkdtempSync(resolve(tmpdir(), "ingest-oracle-"));
    try {
      splitExports(exportString).forEach((payload, index) => {
        expect(round14(decoded[index])).toEqual(round14(luaDecode(payload, scratch)));
      });
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }, 300_000);
});
