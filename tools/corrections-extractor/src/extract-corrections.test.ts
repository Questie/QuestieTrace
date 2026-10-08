import { describe, expect, it } from "vitest";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join, resolve } from "path";
import { extractCorrections } from "./extract-corrections";

const EXAMPLE_TRACE = resolve(__dirname, "../../../Traces/human_rogue_1to5_example.lua");

function setupDirs() {
  const root = mkdtempSync(join(tmpdir(), "corrections-extractor-"));
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
});
