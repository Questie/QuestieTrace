import { afterEach, describe, expect, it, vi } from "vitest";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { TINY_EXPORT } from "./core/trace-data/fixtures";
import { extractCorrections as extract, workersFromEnv, type ExtractOptions } from "./extract-corrections";
import { localSource } from "./store/source";
import * as lookups from "./extract/lookups";
import * as observeLookups from "./extract/observe-lookups";

const tempDirs: string[] = [];

afterEach(() => {
  vi.restoreAllMocks();
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

/** Extracts from the checkout in `traceDataDir`, keeping the observations in a store next to it. */
function extractCorrections(traceDataDir: string, outputDir: string, options: ExtractOptions) {
  return extract(localSource(traceDataDir), join(traceDataDir, "..", "store"), outputDir, options);
}

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
  it("should write all four corrections modules, the quest XP file, the lookups and meta.json to the output directory", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });

    const meta = await extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 });

    expect(readFileSync(join(outputDir, "foreverNpcTraces.lua"), "utf8")).toContain("function ForeverNpcTraces:Load()");
    expect(readFileSync(join(outputDir, "foreverQuestTraces.lua"), "utf8")).toContain("function ForeverQuestTraces:Load()");
    expect(readFileSync(join(outputDir, "foreverItemTraces.lua"), "utf8")).toContain("function ForeverItemTraces:Load()");
    expect(readFileSync(join(outputDir, "foreverObjectTraces.lua"), "utf8")).toContain("function ForeverObjectTraces:Load()");
    expect(readFileSync(join(outputDir, "xpDB-traces.lua"), "utf8")).toContain("---@type QuestXP");
    expect(existsSync(join(outputDir, "lookupNpcs"))).toBe(true);
    expect(JSON.parse(readFileSync(join(outputDir, "meta.json"), "utf8"))).toEqual(meta);
    expect(meta).toMatchObject({ sessionCount: 1, submissionCount: 1, exportCount: 1, skippedFiles: [], partialFiles: [], duplicateSessionsSkipped: 0 });
  });

  it("should replace previous lookups, dropping stale locales, and leave no staging directory", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });
    const stale = writePreviousLookup(outputDir);

    await extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 });

    expect(existsSync(stale)).toBe(false);
    expect(existsSync(join(outputDir, ".lookups-staging"))).toBe(false);
  });

  it("should only write lookups for valid locale names", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });
    vi.spyOn(lookups, "finalizeLookups").mockReturnValue({ lookupItems: { deDE: "de", "../../evil": "evil" } });

    await extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 });

    expect(readFileSync(join(outputDir, "lookupItems", "deDE.lua"), "utf8")).toBe("de");
    expect(existsSync(join(outputDir, "..", "evil.lua"))).toBe(false);
    expect(existsSync(join(outputDir, ".lookups-previous"))).toBe(false);
  });

  it("should skip submissions that fail to load and report them in meta.json", async () => {
    const { traceDataDir, outputDir } = setupDirs({
      "2026-10/2026-10-01_000000Z_a.json": submission("a"),
      "2026-10/2026-10-02_000000Z_broken.json": "{ not json",
      "2026-10/2026-10-03_000000Z_garbled.json": submission("garbled", "not an export"),
    });

    const meta = await extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 });

    expect(meta.submissionCount).toBe(1);
    expect(meta.skippedFiles.map((skipped) => skipped.name)).toEqual([
      join("2026-10", "2026-10-03_000000Z_garbled.json"),
      join("2026-10", "2026-10-02_000000Z_broken.json"),
    ]);
  });

  it("should fold the exports that decode and list the submission in partialFiles", async () => {
    const { traceDataDir, outputDir } = setupDirs({
      "2026-10/2026-10-01_000000Z_a.json": submission("a", `${TINY_EXPORT.slice(0, 200)}${TINY_EXPORT}${TINY_EXPORT.slice(-60)}`),
    });

    const meta = await extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 });

    expect(meta).toMatchObject({ sessionCount: 1, submissionCount: 1, exportCount: 1, skippedFiles: [] });
    expect(meta.partialFiles).toEqual([{ name: join("2026-10", "2026-10-01_000000Z_a.json"), droppedExports: ["export 1 has no matching end marker"] }]);
  });

  it("should fold each session once, even when several submissions and exports contain it", async () => {
    const { traceDataDir, outputDir } = setupDirs({
      "2026-09/2026-09-30_000000Z_a.json": submission("a"),
      "2026-10/2026-10-01_000000Z_b.json": submission("b", `${TINY_EXPORT}\n${TINY_EXPORT}`),
    });

    const meta = await extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 });

    expect(meta).toMatchObject({ sessionCount: 1, submissionCount: 2, exportCount: 3, duplicateSessionsSkipped: 2 });
  });

  it("should throw and keep the previous output when no submission could be loaded", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_broken.json": "{ not json" });
    mkdirSync(outputDir);
    writeFileSync(join(outputDir, "meta.json"), "{}");
    const previous = writePreviousLookup(outputDir);

    await expect(extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 })).rejects.toThrow("could be loaded");
    expect(readFileSync(join(outputDir, "meta.json"), "utf8")).toBe("{}");
    expect(readFileSync(previous, "utf8")).toBe("previous");
    expect(existsSync(join(outputDir, "foreverNpcTraces.lua"))).toBe(false);
  });

  it("should abort and keep the previous output when folding a submission fails", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });
    mkdirSync(outputDir);
    writeFileSync(join(outputDir, "meta.json"), "{}");
    vi.spyOn(observeLookups, "observeLookupSession").mockImplementation(() => {
      throw new Error("observer failed");
    });

    await expect(extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 })).rejects.toThrow("Observing a session of 2026-10/2026-10-01_000000Z_a.json failed (Error)");
    expect(readFileSync(join(outputDir, "meta.json"), "utf8")).toBe("{}");
    expect(existsSync(join(outputDir, "foreverNpcTraces.lua"))).toBe(false);
  });

  it("should not observe older copies of sessions already folded in", async () => {
    const { traceDataDir, outputDir } = setupDirs({
      "2026-09/2026-09-30_000000Z_a.json": submission("a"),
      "2026-10/2026-10-01_000000Z_b.json": submission("b"),
    });
    const observe = vi.spyOn(observeLookups, "observeLookupSession");

    const meta = await extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 });

    expect(meta.duplicateSessionsSkipped).toBe(1);
    expect(observe).toHaveBeenCalledOnce();
  });

  it("should observe a session exported twice in one submission once", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_b.json": submission("b", `${TINY_EXPORT}\n${TINY_EXPORT}`) });
    const observe = vi.spyOn(observeLookups, "observeLookupSession");

    await extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 });

    expect(observe).toHaveBeenCalledOnce();
  });

  it("should reject a worker count that is not a whole number", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });

    await expect(extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: Number.NaN })).rejects.toThrow(/whole number/);
  });

  it("should give the same result with worker threads", async () => {
    const files = {
      "2026-09/2026-09-30_000000Z_a.json": submission("a"),
      "2026-10/2026-10-01_000000Z_b.json": submission("b", `${TINY_EXPORT}\n${TINY_EXPORT}`),
      "2026-10/2026-10-02_000000Z_broken.json": "{ not json",
    };
    const inline = setupDirs(files);
    const threaded = setupDirs(files);

    const { generatedAt: _a, ...inlineMeta } = await extractCorrections(inline.traceDataDir, inline.outputDir, { log: () => undefined, workers: 0 });
    const { generatedAt: _b, ...threadedMeta } = await extractCorrections(threaded.traceDataDir, threaded.outputDir, { log: () => undefined, workers: 2 });

    expect(threadedMeta).toEqual(inlineMeta);
    const modules = (outputDir: string) =>
      ["foreverNpcTraces.lua", "foreverQuestTraces.lua", "foreverItemTraces.lua", "foreverObjectTraces.lua", "xpDB-traces.lua"].map((file) =>
        readFileSync(join(outputDir, file), "utf8").replace(/-- Generated at: .*/, ""),
      );
    expect(modules(threaded.outputDir)).toEqual(modules(inline.outputDir));
    expect(existsSync(join(threaded.outputDir, ".observations"))).toBe(false);
  });

  it("should throw when the trace-data checkout has no submissions directory", async () => {
    const { traceDataDir, outputDir } = setupDirs({});

    await expect(extractCorrections(traceDataDir, outputDir, { log: () => undefined, workers: 0 })).rejects.toThrow("does not exist");
  });
});

describe("extractCorrections (incremental)", () => {
  const options = { log: () => undefined, workers: 0 };

  it("should only observe submissions added since the previous run", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-09/2026-09-30_000000Z_a.json": submission("a") });
    await extractCorrections(traceDataDir, outputDir, options);
    writeFileSync(join(traceDataDir, "submissions", "2026-09", "2026-09-30_000001Z_c.json"), submission("c", "{ not an export"));
    const observe = vi.spyOn(observeLookups, "observeLookupSession");

    const meta = await extractCorrections(traceDataDir, outputDir, options);

    expect(observe).not.toHaveBeenCalled();
    expect(meta).toMatchObject({ sessionCount: 1, submissionCount: 1, skippedFiles: [{ name: "2026-09/2026-09-30_000001Z_c.json" }] });
  });

  it("should use the copy of a session in a newer submission over the stored one", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-09/2026-09-30_000000Z_a.json": submission("a") });
    await extractCorrections(traceDataDir, outputDir, options);
    writeFileSync(join(traceDataDir, "submissions", "2026-09", "2026-09-30_000001Z_b.json"), submission("b"));

    const meta = await extractCorrections(traceDataDir, outputDir, options);

    expect(meta).toMatchObject({ sessionCount: 1, submissionCount: 2, duplicateSessionsSkipped: 1 });
  });

  it("should drop submissions that are no longer in the source", async () => {
    const { traceDataDir, outputDir } = setupDirs({
      "2026-09/2026-09-30_000000Z_a.json": submission("a"),
      "2026-10/2026-10-01_000000Z_b.json": submission("b"),
    });
    await extractCorrections(traceDataDir, outputDir, options);
    rmSync(join(traceDataDir, "submissions", "2026-10"), { recursive: true });

    const meta = await extractCorrections(traceDataDir, outputDir, options);

    expect(meta).toMatchObject({ sessionCount: 1, submissionCount: 1, duplicateSessionsSkipped: 0 });
  });

  it("should try submissions that failed to load again on the next run", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a"), "2026-10/2026-10-02_000000Z_b.json": "{ not json" });
    const first = await extractCorrections(traceDataDir, outputDir, options);
    writeFileSync(join(traceDataDir, "submissions", "2026-10", "2026-10-02_000000Z_b.json"), submission("b"));

    const second = await extractCorrections(traceDataDir, outputDir, options);

    expect(first.skippedFiles).toHaveLength(1);
    expect(second).toMatchObject({ submissionCount: 2, skippedFiles: [] });
  });

  it("should keep the store and the previous output when the source lists no submissions", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });
    await extractCorrections(traceDataDir, outputDir, options);
    const meta = readFileSync(join(outputDir, "meta.json"), "utf8");
    const empty = { list: async () => [], fetch: async () => "", release: () => undefined };

    await expect(extract(empty, join(traceDataDir, "..", "store"), outputDir, options)).rejects.toThrow("lists no submissions");
    expect(readFileSync(join(outputDir, "meta.json"), "utf8")).toBe(meta);
    expect((await extractCorrections(traceDataDir, outputDir, options)).submissionCount).toBe(1);
  });

  it("should reject a batch size that is not a whole number of at least 1", async () => {
    const { traceDataDir, outputDir } = setupDirs({ "2026-10/2026-10-01_000000Z_a.json": submission("a") });

    for (const batchSize of [0, -1, 1.5, Number.NaN]) {
      await expect(extractCorrections(traceDataDir, outputDir, { ...options, batchSize })).rejects.toThrow(/batchSize/);
    }
  });

  it("should give the same result in several runs as in one", async () => {
    const files: Record<string, string> = {
      "2026-09/2026-09-30_000000Z_a.json": submission("a"),
      "2026-10/2026-10-01_000000Z_b.json": submission("b", `${TINY_EXPORT}\n${TINY_EXPORT}`),
      "2026-10/2026-10-02_000000Z_broken.json": "{ not json",
    };
    const once = setupDirs(files);
    const stepwise = setupDirs({});
    for (const [path, content] of Object.entries(files)) {
      mkdirSync(join(stepwise.traceDataDir, "submissions", path, ".."), { recursive: true });
      writeFileSync(join(stepwise.traceDataDir, "submissions", path), content);
      await extractCorrections(stepwise.traceDataDir, stepwise.outputDir, options);
    }

    const { generatedAt: _a, ...onceMeta } = await extractCorrections(once.traceDataDir, once.outputDir, { ...options, batchSize: 1 });
    const { generatedAt: _b, ...stepwiseMeta } = await extractCorrections(stepwise.traceDataDir, stepwise.outputDir, options);

    expect(stepwiseMeta).toEqual(onceMeta);
    const modules = (outputDir: string) =>
      ["foreverNpcTraces.lua", "foreverQuestTraces.lua", "foreverItemTraces.lua", "foreverObjectTraces.lua", "xpDB-traces.lua"].map((file) =>
        readFileSync(join(outputDir, file), "utf8").replace(/-- Generated at: .*/, ""),
      );
    expect(modules(stepwise.outputDir)).toEqual(modules(once.outputDir));
  });
});

describe("workersFromEnv", () => {
  it("should use the default when EXTRACT_WORKERS is not set", () => {
    expect(workersFromEnv({})).toBeUndefined();
  });

  it("should read the number of workers from EXTRACT_WORKERS", () => {
    expect(workersFromEnv({ EXTRACT_WORKERS: "4" })).toBe(4);
    expect(workersFromEnv({ EXTRACT_WORKERS: "0" })).toBe(0);
  });

  it("should reject an EXTRACT_WORKERS that is not a whole number of at least 0", () => {
    for (const value of ["abc", "-1", "1.5", "", "4\n", "9".repeat(400), "9007199254740993"]) {
      expect(() => workersFromEnv({ EXTRACT_WORKERS: value })).toThrow("EXTRACT_WORKERS");
    }
  });
});
