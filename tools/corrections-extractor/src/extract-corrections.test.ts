import { afterEach, describe, expect, it, vi } from "vitest";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { TINY_EXPORT } from "./core/trace-data/fixtures";
import { extractCorrections } from "./extract-corrections";
import * as lookups from "./extract/lookups";

const tempDirs: string[] = [];

afterEach(() => {
  vi.restoreAllMocks();
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

/** A trace-data checkout holding `files` (path under submissions/ -> content), plus an output directory. */
function setupDirs(files: Record<string, string>) {
  const root = mkdtempSync(join(tmpdir(), "corrections-extractor-"));
  tempDirs.push(root);
  const traceDataDir = join(root, "trace-data");
  const outputDir = join(root, "output");
  for (const [path, content] of Object.entries(files)) {
    const full = join(traceDataDir, "submissions", path);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, content);
  }
  return { traceDataDir, outputDir };
}

function submission(id: string, exportString = TINY_EXPORT): string {
  return JSON.stringify({ id, contributor_id: "contributor", received_at: "2026-10-01T00:00:00Z", export_string: exportString });
}

function writePreviousLookup(outputDir: string): string {
  const file = join(outputDir, "lookupItems", "deDE.lua");
  mkdirSync(join(outputDir, "lookupItems"), { recursive: true });
  writeFileSync(file, "previous");
  return file;
}

describe("extractCorrections", () => {
  it("should write all four corrections modules, the lookups and meta.json to the output directory", () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });

    const meta = extractCorrections(traceDataDir, outputDir, () => undefined);

    expect(readFileSync(join(outputDir, "foreverNpcTraces.lua"), "utf8")).toContain("function ForeverNpcTraces:Load()");
    expect(readFileSync(join(outputDir, "foreverQuestTraces.lua"), "utf8")).toContain("function ForeverQuestTraces:Load()");
    expect(readFileSync(join(outputDir, "foreverItemTraces.lua"), "utf8")).toContain("function ForeverItemTraces:Load()");
    expect(readFileSync(join(outputDir, "foreverObjectTraces.lua"), "utf8")).toContain("function ForeverObjectTraces:Load()");
    expect(existsSync(join(outputDir, "lookupNpcs"))).toBe(true);
    expect(JSON.parse(readFileSync(join(outputDir, "meta.json"), "utf8"))).toEqual(meta);
    expect(meta).toMatchObject({ sessionCount: 1, submissionCount: 1, exportCount: 1, skippedFiles: [], duplicateSessionsSkipped: 0 });
  });

  it("should replace previous lookups, dropping stale locales, and leave no staging directory", () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });
    const stale = writePreviousLookup(outputDir);

    extractCorrections(traceDataDir, outputDir, () => undefined);

    expect(existsSync(stale)).toBe(false);
    expect(existsSync(join(outputDir, ".lookups-staging"))).toBe(false);
  });

  it("should skip submissions that fail to load and report them in meta.json", () => {
    const { traceDataDir, outputDir } = setupDirs({
      "2026-10/2026-10-01_000000Z_a.json": submission("a"),
      "2026-10/2026-10-02_000000Z_broken.json": "{ not json",
      "2026-10/2026-10-03_000000Z_garbled.json": submission("garbled", "not an export"),
    });

    const meta = extractCorrections(traceDataDir, outputDir, () => undefined);

    expect(meta.submissionCount).toBe(1);
    expect(meta.skippedFiles.map((skipped) => skipped.name)).toEqual([
      join("2026-10", "2026-10-03_000000Z_garbled.json"),
      join("2026-10", "2026-10-02_000000Z_broken.json"),
    ]);
  });

  it("should fold each session once, even when several submissions and exports contain it", () => {
    const { traceDataDir, outputDir } = setupDirs({
      "2026-09/2026-09-30_000000Z_a.json": submission("a"),
      "2026-10/2026-10-01_000000Z_b.json": submission("b", `${TINY_EXPORT}\n${TINY_EXPORT}`),
    });

    const meta = extractCorrections(traceDataDir, outputDir, () => undefined);

    expect(meta).toMatchObject({ sessionCount: 1, submissionCount: 2, exportCount: 3, duplicateSessionsSkipped: 2 });
  });

  it("should throw and keep the previous output when no submission could be loaded", () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_broken.json": "{ not json" });
    mkdirSync(outputDir);
    writeFileSync(join(outputDir, "meta.json"), "{}");
    const previous = writePreviousLookup(outputDir);

    expect(() => extractCorrections(traceDataDir, outputDir, () => undefined)).toThrow("could be loaded");
    expect(readFileSync(join(outputDir, "meta.json"), "utf8")).toBe("{}");
    expect(readFileSync(previous, "utf8")).toBe("previous");
    expect(existsSync(join(outputDir, "foreverNpcTraces.lua"))).toBe(false);
  });

  it("should abort and keep the previous output when folding a submission fails", () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });
    mkdirSync(outputDir);
    writeFileSync(join(outputDir, "meta.json"), "{}");
    vi.spyOn(lookups, "foldLookupSessions").mockImplementation(() => {
      throw new Error("observer failed");
    });

    expect(() => extractCorrections(traceDataDir, outputDir, () => undefined)).toThrow("observer failed");
    expect(readFileSync(join(outputDir, "meta.json"), "utf8")).toBe("{}");
    expect(existsSync(join(outputDir, "foreverNpcTraces.lua"))).toBe(false);
  });

  it("should throw when the trace-data checkout has no submissions directory", () => {
    const { traceDataDir, outputDir } = setupDirs({});

    expect(() => extractCorrections(traceDataDir, outputDir, () => undefined)).toThrow("does not exist");
  });
});
