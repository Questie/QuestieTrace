import { afterEach, describe, expect, it } from "vitest";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join, resolve } from "path";
import { extractCorrections } from "./extract-corrections";

const EXAMPLE_TRACE = resolve(__dirname, "../../../Traces/human_rogue_1to5_example.lua");

const tempDirs: string[] = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

function setupDirs() {
  const root = mkdtempSync(join(tmpdir(), "corrections-extractor-"));
  tempDirs.push(root);
  const traceDir = join(root, "Traces");
  const outputDir = join(root, "output");
  mkdirSync(traceDir);
  return { traceDir, outputDir };
}

describe("extractCorrections", () => {
  it("should write all four corrections modules and meta.json to the output directory", () => {
    const { traceDir, outputDir } = setupDirs();
    copyFileSync(EXAMPLE_TRACE, join(traceDir, "example.lua"));

    const meta = extractCorrections(traceDir, outputDir, () => undefined);

    expect(readFileSync(join(outputDir, "foreverNpcTraces.lua"), "utf8")).toContain("function ForeverNpcTraces:Load()");
    expect(readFileSync(join(outputDir, "foreverQuestTraces.lua"), "utf8")).toContain("function ForeverQuestTraces:Load()");
    expect(readFileSync(join(outputDir, "foreverItemTraces.lua"), "utf8")).toContain("function ForeverItemTraces:Load()");
    expect(readFileSync(join(outputDir, "foreverObjectTraces.lua"), "utf8")).toContain("function ForeverObjectTraces:Load()");
    expect(JSON.parse(readFileSync(join(outputDir, "meta.json"), "utf8"))).toEqual(meta);
    expect(meta.fileCount).toBe(1);
    expect(meta.skippedFiles).toEqual([]);
  });

  it("should skip trace files that fail to load and report them in meta.json", () => {
    const { traceDir, outputDir } = setupDirs();
    writeFileSync(join(traceDir, "broken.lua"), "this is not lua {");
    writeFileSync(join(traceDir, "ignored.txt"), "not a trace file");

    const meta = extractCorrections(traceDir, outputDir, () => undefined);

    expect(meta.fileCount).toBe(0);
    expect(meta.skippedFiles).toHaveLength(1);
    expect(meta.skippedFiles[0].name).toBe("broken.lua");
  });

  it("should throw when the trace directory does not exist", () => {
    const { traceDir, outputDir } = setupDirs();
    rmSync(traceDir, { recursive: true });

    expect(() => extractCorrections(traceDir, outputDir, () => undefined)).toThrow("Trace directory not found");
  });

  it("should throw and keep the previous output when the trace directory has no trace files", () => {
    const { traceDir, outputDir } = setupDirs();
    mkdirSync(outputDir);
    writeFileSync(join(outputDir, "meta.json"), "{}");
    writeFileSync(join(traceDir, "ignored.txt"), "not a trace file");

    expect(() => extractCorrections(traceDir, outputDir, () => undefined)).toThrow("No .lua trace files found");
    expect(readFileSync(join(outputDir, "meta.json"), "utf8")).toBe("{}");
    expect(existsSync(join(outputDir, "foreverNpcTraces.lua"))).toBe(false);
  });
});
