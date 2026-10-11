import { dirname, resolve } from "path";
import { readFileSync } from "fs";
import { fileURLToPath } from "url";
import { describe, expect, it } from "vitest";
import { cacheVersionOf, pipelineSources } from "./submission";

const here = dirname(fileURLToPath(import.meta.url));
const entry = resolve(here, "submission.ts");

describe("ingest cache key", () => {
  it("covers every module that shapes cached results, found by following imports", () => {
    const sources = [...pipelineSources(entry).keys()];
    const shared = ["submissions.ts", "decode.ts", "cbor.ts", "session-key.ts"].map((name) => `../../core/trace-data/${name}`);
    for (const module of [...shared, "distill.ts", "streams.ts", "../../core/normalize.ts", "../../core/guid.ts"]) {
      expect(sources).toContain(resolve(here, module));
    }
  });

  it("changes when any of those modules changes", () => {
    const current = cacheVersionOf(pipelineSources(entry), here);
    const editedCbor = pipelineSources(entry, (path) => readFileSync(path, "utf8") + (path.endsWith("cbor.ts") ? "\n// edited" : ""));
    expect(cacheVersionOf(editedCbor, here)).not.toBe(current);
    expect(cacheVersionOf(pipelineSources(entry), here)).toBe(current);
  });
});
