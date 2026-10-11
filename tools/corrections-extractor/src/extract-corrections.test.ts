import { afterEach, describe, expect, it } from "vitest";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join, resolve } from "path";
import { extractCorrections, extractCorrectionsFromTraceData, parseExtractArgs } from "./extract-corrections";

const EXAMPLE_TRACE = resolve(__dirname, "../../../Traces/human_rogue_1to5_example.lua");

// Synthetic exports (no real trace data), built like core/trace-data/fixtures.ts. OLDER and NEWER
// both hold a copy of one session (same capture-start clocks) whose title for quest 40001 differs
// per copy; NEWER also holds a second session with quest 40002. FAILING is NEWER plus a third
// session without `functions`, which the extractor throws on. CLOCKLESS holds one session without
// capture-start clocks twice.
const OLDER_EXPORT =
  "!QuestieTrace:1!94Ctokiquqayz5Cdg35jW7qRish)dNrzRPttHssBdPFpiXDW9WnmtI3r3j9g2vPYxsv9XeiQO0sDVsiw5yKj4eh0feENglh7(otMxB1S3(NCn4TLALb9rvg1d46(xWtwWvtenHWE(3FwE5ynOb115dyPnVSF6tHNfmWcen9R4c2GnhmzWn7KpVFCHqyyv7osFd3vPW5V4xrObwMA)a!End:QuestieTrace:1!";
const NEWER_EXPORT =
  "!QuestieTrace:1!BHEuCQfxCM5NxXnTSakUKelQKutXXscOOutoZIt9NH5HNWftIu8mTsZl5saP2f5P7PwIp5NCI5K6cDRGCsSYulQXf7yjm4ujfWGJL5sQ5fAWb5C8bwAQfduvPRhqf7zEPL)cLCoocszmcuzm6yzl01sYSKCs1h)sTCGQtHqaXXp4wHlPMtjj2GVfNCgPMBIHLArGCJC6wQLLAELuCdyXPgLli5upbvWP6eMovV9VKmsTifaRyI0Lc4p!End:QuestieTrace:1!";
const FAILING_EXPORT =
  "!QuestieTrace:1!BHEuCQfxCM5NxXnVSakUKelQKutXXscOOutoZIt9NH5HNWftIu8mTsZl5saP2f5P7PwIp5NCI5K6cDRGCsSYulQXf7yjm4ujfWGJL5sQ5fAWb5C8bwAQfduvPRhqf7zEPL)cLCoocszmcuzm6yzl01sYSKCs1h)sTCGQtHqaXXp4wHlPMtjj2GVfNCgPMBIHLArGCJC6wQLLAELuCdyXPgLli5upbvWP6eMovV9VKmsTifaRyI0LUeymW0fhZMqtpi8asYOoac!End:QuestieTrace:1!";
const CLOCKLESS_EXPORT =
  "!QuestieTrace:1!BHEuCQfxCM5NxXnTeptR08sUeqSxKNUNAj(KFYjMtQl0TcYjXktTOgxSJLWGtLuadowMlPMxObhKZXhyPPwmqvLUEavSN5Lw(luY54miLXiqLXOJLTqxljZsYjv)DoN8to7CaAlkawd(b3wCj1CkjXg8T4KZi1CtmSulcKZGt3sTSuZRKIByWLRbWd!End:QuestieTrace:1!";

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

describe("extractCorrectionsFromTraceData", () => {
  function setupCheckout(submissions: Record<string, object>) {
    const { traceDir, outputDir } = setupDirs();
    for (const [path, content] of Object.entries(submissions)) {
      mkdirSync(join(traceDir, "submissions", path, ".."), { recursive: true });
      writeFileSync(join(traceDir, "submissions", path), JSON.stringify(content));
    }
    return { traceDataDir: traceDir, outputDir };
  }

  it("should fold only the newest copy of a session that several submissions hold", () => {
    const { traceDataDir, outputDir } = setupCheckout({
      "2026-09/2026-09-30_120000Z_older.json": { id: "older", export_string: OLDER_EXPORT },
      "2026-10/2026-10-01_120000Z_newer.json": { id: "newer", export_string: NEWER_EXPORT },
      "2026-10/2026-10-02_120000Z_broken.json": { id: "broken" },
    });

    const meta = extractCorrectionsFromTraceData(traceDataDir, outputDir, () => undefined);

    const quests = readFileSync(join(outputDir, "foreverQuestTraces.lua"), "utf8");
    expect(quests).toContain('"Newest Title"');
    expect(quests).toContain('"Other Quest"');
    expect(quests).not.toContain("Older Title");
    expect(meta).toMatchObject({ source: "trace-data", sessionCount: 2, fileCount: 2, submissionCount: 2, duplicateSessionsSkipped: 1 });
    expect(meta.skippedFiles).toEqual([{ name: join("2026-10", "2026-10-02_120000Z_broken.json"), error: "no export_string" }]);
    expect(JSON.parse(readFileSync(join(outputDir, "meta.json"), "utf8"))).toEqual(meta);
  });

  it("should leave nothing of a submission an observer fails on and fold its sessions' older copies instead", () => {
    const { traceDataDir, outputDir } = setupCheckout({
      "2026-09/2026-09-30_120000Z_older.json": { id: "older", export_string: OLDER_EXPORT },
      "2026-10/2026-10-01_120000Z_failing.json": { id: "failing", export_string: FAILING_EXPORT },
    });

    const meta = extractCorrectionsFromTraceData(traceDataDir, outputDir, () => undefined);

    const quests = readFileSync(join(outputDir, "foreverQuestTraces.lua"), "utf8");
    expect(quests).toContain('"Older Title"');
    expect(quests).not.toContain("Newest Title");
    expect(quests).not.toContain("Other Quest");
    expect(meta).toMatchObject({ sessionCount: 1, submissionCount: 1, duplicateSessionsSkipped: 0 });
    // Only the error's name: its message could quote trace data.
    expect(meta.skippedFiles).toEqual([{ name: join("2026-10", "2026-10-01_120000Z_failing.json"), error: "folding the sessions failed (TypeError)" }]);
  });

  it("should fold every copy of a session without capture-start clocks, since its copies cannot be matched", () => {
    const { traceDataDir, outputDir } = setupCheckout({
      "2026-10/2026-10-01_120000Z_clockless.json": { id: "clockless", export_string: CLOCKLESS_EXPORT },
    });

    const meta = extractCorrectionsFromTraceData(traceDataDir, outputDir, () => undefined);

    expect(meta).toMatchObject({ sessionCount: 2, duplicateSessionsSkipped: 0 });
  });
});

describe("parseExtractArgs", () => {
  const defaults = { traceDir: "/repo/Traces", traceDataDir: "/repo/trace-data" };

  it("should read Traces/ by default and a trace-data checkout with --trace-data [dir]", () => {
    expect(parseExtractArgs([], defaults)).toEqual({ source: "traces", dir: "/repo/Traces" });
    expect(parseExtractArgs(["--trace-data"], defaults)).toEqual({ source: "trace-data", dir: "/repo/trace-data" });
    expect(parseExtractArgs(["--trace-data", "/elsewhere"], defaults)).toEqual({ source: "trace-data", dir: "/elsewhere" });
  });

  it("should reject anything else instead of silently reading Traces/", () => {
    for (const args of [["--tracedata"], ["../trace-data"], ["--trace-data", ""], ["--trace-data", "a", "b"], ["--trace-data", "--force"]]) {
      expect(() => parseExtractArgs(args, defaults)).toThrow(/Usage/);
    }
  });
});
