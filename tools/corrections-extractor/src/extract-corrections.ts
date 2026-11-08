// Runs the corrections and lookup extraction over every submission in a trace-data checkout
// and writes the forever*Traces.lua modules, one lookup<Entity>/<locale>.lua module per
// entity and locale, plus a meta.json summary.
//
// Submissions are decoded and observed in worker threads, then folded in one at a time, in
// order, and dropped. Raw observations are kept on disk until finalize (see
// ObservationStore), so memory use stays about flat as submissions come in.
//
// Exports are cumulative and include the in-progress session, so one session shows up in
// many submissions, growing as play continues. Submissions are read newest first and only
// the first copy of each session met (the newest, so the most complete: see
// core/trace-data/session-key.ts) is folded in.

import { existsSync, mkdirSync, renameSync, rmSync, writeFileSync } from "fs";
import { join, relative, resolve } from "path";
import { availableParallelism } from "os";
import { Worker } from "worker_threads";
import { listSubmissionFiles } from "./core/trace-data/submissions";
import { addStagedSessions, createAccumulator, finalize } from "./extract";
import { addStagedLookupSessions, createLookupAccumulator, finalizeLookups, type LookupBundle } from "./extract/lookups";
import { observeSubmission, type ObservedSession, type ObservedSubmission, type ObserveRequest } from "./observe-submission";

const OUTPUT_FILES = {
  npcFixes: "foreverNpcTraces.lua",
  questFixes: "foreverQuestTraces.lua",
  itemFixes: "foreverItemTraces.lua",
  objectFixes: "foreverObjectTraces.lua",
  questXp: "xpDB-traces.lua",
} as const;

const LOCALE_PATTERN = /^[a-z]{2}[A-Z]{2}$/;

export interface ExtractMeta {
  sessionCount: number;
  /** Submissions folded in. */
  submissionCount: number;
  /** Exports decoded from the folded submissions; a submission may hold several. */
  exportCount: number;
  /** Submission files, relative to submissions/, that failed to load. */
  skippedFiles: { name: string; error: string }[];
  /** Submission files, relative to submissions/, of which only some exports could be decoded. */
  partialFiles: { name: string; droppedExports: string[] }[];
  /** Session copies skipped because a newer copy was already folded in. */
  duplicateSessionsSkipped: number;
  generatedAt: string;
}

export interface ExtractOptions {
  log?: (message: string) => void;
  /** Worker threads that read and observe submissions; 0 does it on the main thread. */
  workers?: number;
}

/** EXTRACT_WORKERS as the number of worker threads, or undefined for the default. */
export function workersFromEnv(env: NodeJS.ProcessEnv = process.env): number | undefined {
  const value = env.EXTRACT_WORKERS;
  if (value === undefined) return undefined;
  if (!/^\d+$/.test(value)) throw new Error(`EXTRACT_WORKERS must be a whole number of at least 0 (got "${value}")`);
  return Number(value);
}

export async function extractCorrections(
  traceDataDir: string,
  outputDir: string,
  { log = console.log, workers = Math.max(1, availableParallelism() - 1) }: ExtractOptions = {},
): Promise<ExtractMeta> {
  // Newest first: the first copy of a session met is then its most complete one.
  const files = listSubmissionFiles(traceDataDir).reverse();
  const submissionsDir = resolve(traceDataDir, "submissions");

  // Raw observations go to disk, so memory use does not grow with the number of submissions.
  const storeDir = join(outputDir, ".observations");
  const acc = createAccumulator({}, storeDir);
  const lookupAcc = createLookupAccumulator();
  const foldedKeys = new Set<string>();
  const foldedSubmissionIds: string[] = [];
  const skippedFiles: ExtractMeta["skippedFiles"] = [];
  const partialFiles: ExtractMeta["partialFiles"] = [];
  let exportCount = 0;
  let duplicateSessionsSkipped = 0;

  try {
    await observeInOrder(files.map((file) => file.path), acc.maxIds, workers, foldedKeys, (file, submission, index) => {
      const name = relative(submissionsDir, file);
      const keys = new Set<string>();
      if (submission.error) {
        skippedFiles.push({ name, error: submission.error });
      } else {
        const sessions: ObservedSession[] = [];
        let duplicates = 0;
        for (const session of submission.sessions) {
          // A session without a key cannot be matched with its copies, so every copy is folded in.
          if (session.key !== undefined && (foldedKeys.has(session.key) || keys.has(session.key))) {
            duplicates++;
          } else {
            if (session.key !== undefined) keys.add(session.key);
            sessions.push(session);
          }
        }
        // Observer errors abort the run, keeping the previous output: a submission folded in
        // only part way could not be taken out of the accumulators again.
        const failed = sessions.find((session) => session.error !== undefined);
        if (failed) throw new Error(`Observing a session of ${name} failed (${failed.error})`);
        addStagedSessions(acc, sessions.flatMap((session) => (session.extract ? [session.extract] : [])));
        addStagedLookupSessions(lookupAcc, sessions.flatMap((session) => (session.lookup ? [session.lookup] : [])));
        for (const key of keys) foldedKeys.add(key);
        duplicateSessionsSkipped += duplicates;
        foldedSubmissionIds.push(submission.submissionId);
        exportCount += submission.exportCount;
        if (submission.droppedExports.length > 0) partialFiles.push({ name, droppedExports: submission.droppedExports });
      }
      if ((index + 1) % 250 === 0 || index + 1 === files.length) log(`[${index + 1}/${files.length}] submissions`);
      return keys;
    });
    return writeOutputs();
  } finally {
    acc.observations.dispose();
  }

  function writeOutputs(): ExtractMeta {
    if (foldedSubmissionIds.length === 0) {
      throw new Error(`No submission in ${submissionsDir} could be loaded; keeping the previous output`);
    }

    const now = new Date();
    const bundle = finalize(acc, { sourceFileNames: foldedSubmissionIds, now });

    mkdirSync(outputDir, { recursive: true });
    // meta.json marks a complete run: remove it until all modules are written.
    rmSync(join(outputDir, "meta.json"), { force: true });
    for (const [field, fileName] of Object.entries(OUTPUT_FILES)) {
      writeFileSync(join(outputDir, fileName), bundle[field as keyof typeof OUTPUT_FILES]);
    }
    writeLookups(finalizeLookups(lookupAcc), outputDir);

    const meta: ExtractMeta = {
      sessionCount: acc.sessionCount,
      submissionCount: foldedSubmissionIds.length,
      exportCount,
      skippedFiles,
      partialFiles,
      duplicateSessionsSkipped,
      generatedAt: now.toISOString(),
    };
    writeFileSync(join(outputDir, "meta.json"), JSON.stringify(meta, null, 2));
    return meta;
  }
}

/**
 * Observes every file, `workers` at a time, and hands the results to `fold` in the order of
 * `files`. At most a few results wait for an earlier one, so memory use stays bounded.
 */
async function observeInOrder(
  files: string[],
  maxIds: ObserveRequest["maxIds"],
  workers: number,
  usedKeys: ReadonlySet<string>,
  fold: (file: string, submission: ObservedSubmission, index: number) => Set<string> | void,
): Promise<void> {
  // NaN would create no workers and never finish.
  if (!Number.isInteger(workers)) throw new Error(`workers must be a whole number (got ${workers})`);
  if (workers <= 0) {
    files.forEach((file, index) => fold(file, observeSubmission({ path: file, maxIds }, usedKeys), index));
    return;
  }

  const pool = Array.from({ length: Math.min(workers, files.length) }, () => new Worker(new URL("./observe-worker.ts", import.meta.url), { execArgv: ["--import", import.meta.resolve("tsx")] }));
  const results = new Map<number, ObservedSubmission>();
  let next = 0;
  let folded = 0;
  let stopped = false;
  try {
    await new Promise<void>((resolveAll, rejectAll) => {
      const dispatch = (worker: Worker) => {
        // Workers stop taking files while too many results wait to be folded.
        if (stopped || next >= files.length || next - folded >= pool.length * 4) return false;
        const index = next++;
        worker.once("message", (result: ObservedSubmission) => {
          results.set(index, result);
          try {
            while (results.has(folded)) {
              const ready = results.get(folded)!;
              results.delete(folded);
              const keys = fold(files[folded], ready, folded);
              folded++;
              // Older submissions still to be observed hold copies of these sessions: skip them there.
              if (keys && keys.size > 0) for (const other of pool) other.postMessage({ usedKeys: [...keys] });
            }
          } catch (e) {
            stopped = true;
            rejectAll(e);
            return;
          }
          if (folded === files.length) resolveAll();
          for (const idle of pool) if (idle.listenerCount("message") === 0) dispatch(idle);
        });
        worker.postMessage({ path: files[index], maxIds } satisfies ObserveRequest);
        return true;
      };
      for (const worker of pool) {
        worker.on("error", (e) => {
          stopped = true;
          rejectAll(e);
        });
        // A worker that exits without an error would otherwise leave its file unfolded forever.
        worker.on("exit", (code) => {
          stopped = true;
          rejectAll(new Error(`An observe worker exited early (code ${code})`));
        });
        dispatch(worker);
      }
      if (files.length === 0) resolveAll();
    });
  } finally {
    await Promise.all(pool.map((worker) => worker.terminate()));
  }
}

function writeLookups(bundle: LookupBundle, outputDir: string): void {
  // Write everything to a staging directory first, so a failed write keeps the previous lookups.
  const stagingDir = join(outputDir, ".lookups-staging");
  rmSync(stagingDir, { recursive: true, force: true });
  try {
    for (const [dirName, files] of Object.entries(bundle)) {
      const dir = join(stagingDir, dirName);
      mkdirSync(dir, { recursive: true });
      for (const [locale, content] of Object.entries(files)) {
        // Locales come from trace data; anything but a WoW locale name could leave the directory.
        if (!LOCALE_PATTERN.test(locale)) continue;
        writeFileSync(join(dir, `${locale}.lua`), content);
      }
    }
    // Replacing whole directories also drops stale locales from a previous run. The previous
    // ones are moved aside first and moved back when any rename fails.
    const previousDir = join(outputDir, ".lookups-previous");
    rmSync(previousDir, { recursive: true, force: true });
    mkdirSync(previousDir);
    const dirNames = Object.keys(bundle);
    try {
      for (const dirName of dirNames) {
        if (existsSync(join(outputDir, dirName))) renameSync(join(outputDir, dirName), join(previousDir, dirName));
      }
      for (const dirName of dirNames) renameSync(join(stagingDir, dirName), join(outputDir, dirName));
    } catch (e) {
      for (const dirName of dirNames) {
        if (!existsSync(join(previousDir, dirName))) continue;
        rmSync(join(outputDir, dirName), { recursive: true, force: true });
        renameSync(join(previousDir, dirName), join(outputDir, dirName));
      }
      throw e;
    } finally {
      rmSync(previousDir, { recursive: true, force: true });
    }
  } finally {
    rmSync(stagingDir, { recursive: true, force: true });
  }
}
