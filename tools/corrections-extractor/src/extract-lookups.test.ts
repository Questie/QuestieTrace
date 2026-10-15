import { afterEach, describe, expect, it } from "vitest";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join, resolve } from "path";
import { extractLookups } from "./extract-lookups";

const EXAMPLE_TRACE = resolve(__dirname, "../../../Traces/human_rogue_1to5_example.lua");

const tempDirs: string[] = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

function setupDirs() {
  const root = mkdtempSync(join(tmpdir(), "lookups-extractor-"));
  tempDirs.push(root);
  const traceDir = join(root, "Traces");
  const outputDir = join(root, "output");
  mkdirSync(traceDir);
  return { traceDir, outputDir };
}

function writePreviousLookup(outputDir: string): string {
  const file = join(outputDir, "lookupItems", "deDE.lua");
  mkdirSync(join(outputDir, "lookupItems"), { recursive: true });
  writeFileSync(file, "previous");
  return file;
}

describe("extractLookups", () => {
  it("should replace previous lookups, dropping stale locales, and leave no staging directory", () => {
    const { traceDir, outputDir } = setupDirs();
    copyFileSync(EXAMPLE_TRACE, join(traceDir, "example.lua"));
    const stale = writePreviousLookup(outputDir);

    const meta = extractLookups(traceDir, outputDir, () => undefined);

    expect(meta.loadedFileNames).toEqual(["example.lua"]);
    expect(existsSync(stale)).toBe(false);
    expect(existsSync(join(outputDir, "lookupNpcs"))).toBe(true);
    expect(existsSync(join(outputDir, ".lookups-staging"))).toBe(false);
  });

  it("should throw and keep the previous lookups when no trace file could be loaded", () => {
    const { traceDir, outputDir } = setupDirs();
    writeFileSync(join(traceDir, "broken.lua"), "this is not lua {");
    const previous = writePreviousLookup(outputDir);

    expect(() => extractLookups(traceDir, outputDir, () => undefined)).toThrow("could be loaded");
    expect(readFileSync(previous, "utf8")).toBe("previous");
  });
});
