// Runs the corrections and lookup extraction over every submission of a source (see
// store/source.ts) and writes the forever*Traces.lua modules, one lookup<Entity>/<locale>.lua
// module per entity and locale, plus a meta.json summary.
//
// Only submissions not yet in the store (see store/observed-store.ts) are fetched, in batches,
// decoded and observed in worker threads; their observations are added to the store and the
// fetched files dropped. Then every stored observation is folded in and finalized. Raw
// observations of fields with a custom merge are kept on disk until finalize (see
// ObservationStore), so memory use stays about flat.
//
// Exports are cumulative and include the in-progress session, so one session shows up in
// many submissions, growing as play continues. Submissions are folded newest first and only
// the first copy of each session met (the newest, so the most complete: see
// core/trace-data/session-key.ts) is used.

import { existsSync, mkdirSync, renameSync, rmSync, writeFileSync } from "fs";
import { join, relative, resolve } from "path";
import { availableParallelism } from "os";
import { Worker } from "worker_threads";
import { addStagedSessions, createAccumulator, finalize, MAX_IDS } from "./extract";
import { addStagedLookupSessions, createLookupAccumulator, finalizeLookups, type LookupBundle } from "./extract/lookups";
import { observeSubmission, type ObservedSubmission, type ObserveRequest } from "./observe-submission";
import { observerVersion } from "./store/observer-version";
import { ObservedStore, type StoredSubmission } from "./store/observed-store";
import type { SubmissionSource } from "./store/source";

/** Submissions fetched and stored at a time. */
const BATCH_SIZE = 500;

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
  /** Submissions fetched and stored at a time. */
  batchSize?: number;
}

/** EXTRACT_WORKERS as the number of worker threads, or undefined for the default. */
export function workersFromEnv(env: NodeJS.ProcessEnv = process.env): number | undefined {
  const value = env.EXTRACT_WORKERS;
  if (value === undefined) return undefined;
  const workers = Number(value);
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(workers)) {
    throw new Error(`EXTRACT_WORKERS must be a whole number from 0 to Number.MAX_SAFE_INTEGER (got "${value}")`);
  }
  return workers;
}

export async function extractCorrections(
  source: SubmissionSource,
  storeDir: string,
  outputDir: string,
  { log = console.log, workers = Math.max(1, availableParallelism() - 1), batchSize = BATCH_SIZE }: ExtractOptions = {},
): Promise<ExtractMeta> {
  const store = await ObservedStore.open(storeDir, observerVersion());
  try {
    await updateStore(store, source, { log, workers, batchSize });
    return await writeOutputs(store, outputDir, log);
  } finally {
    store.close();
  }
}

/** Observes the submissions the store does not have yet and drops those the source no longer has. */
async function updateStore(store: ObservedStore, source: SubmissionSource, { log, workers, batchSize }: Required<ExtractOptions>): Promise<void> {
  // NaN would create no workers and never finish.
  if (!Number.isInteger(workers)) throw new Error(`workers must be a whole number (got ${workers})`);
  const keys = await source.list();
  const listed = new Set(keys);
  const gone = [...store.keys()].filter((key) => !listed.has(key));
  if (gone.length > 0) {
    // Other submissions may hold copies of the dropped ones' sessions that were not observed
    // (see ObservedSession.duplicate): observe them again.
    const redo = (await store.holdingCopiesOf(gone)).filter((key) => listed.has(key));
    store.forget([...gone, ...redo]);
    log(`Dropped ${gone.length} submission(s) no longer in the source; observing ${redo.length} older one(s) again`);
  }
  const stored = store.keys();
  // Newest first: the first copy of a session met is then its most complete one.
  const added = keys.filter((key) => !stored.has(key)).sort().reverse();
  log(`${stored.size} submission(s) already observed, ${added.length} new`);

  const usedKeys = new Set<string>();
  const pool = Array.from({ length: Math.max(0, Math.min(workers, added.length)) }, () => new Worker(new URL("./observe-worker.ts", import.meta.url), { execArgv: ["--import", import.meta.resolve("tsx")] }));
  try {
    await observeBatches();
  } finally {
    await Promise.all(pool.map((worker) => worker.terminate()));
  }

  async function observeBatches(): Promise<void> {
    for (let start = 0; start < added.length; start += batchSize) {
      const batch = added.slice(start, start + batchSize);
      const dir = await source.fetch(batch);
      try {
        const observed: StoredSubmission[] = [];
        await observeInOrder(batch.map((key) => join(dir, key)), MAX_IDS, pool, usedKeys, (_file, submission, index) => {
          observed.push({ key: batch[index], submission });
          const keys = new Set<string>();
          for (const session of submission.sessions) if (session.key !== undefined && !session.duplicate) keys.add(session.key);
          for (const key of keys) usedKeys.add(key);
          return keys;
        });
        await store.add(observed);
      } finally {
        source.release(dir);
      }
      log(`[${Math.min(start + batchSize, added.length)}/${added.length}] new submissions observed`);
    }
  }
}

async function writeOutputs(store: ObservedStore, outputDir: string, log: (message: string) => void): Promise<ExtractMeta> {
  const entries = store.entries().sort(([a], [b]) => (a < b ? 1 : -1));
  const loaded = entries.filter(([, entry]) => entry.error === undefined);
  if (loaded.length === 0) throw new Error("No submission could be loaded; keeping the previous output");

  // Raw observations go to disk, so memory use does not grow with the number of submissions.
  const acc = createAccumulator({}, join(outputDir, ".observations"));
  const lookupAcc = createLookupAccumulator();
  try {
    log("Folding stored observations");
    const { duplicateCopies } = await store.replay((session) => {
      // Observer errors abort the run, keeping the previous output.
      if (session.error !== undefined) throw new Error(`Observing a session of ${session.submissionKey} failed (${session.error})`);
      if (session.extract) addStagedSessions(acc, [session.extract]);
      if (session.lookup) addStagedLookupSessions(lookupAcc, [session.lookup]);
    }, (done, total) => log(`[${done}/${total}] submissions folded`));

    const now = new Date();
    const bundle = finalize(acc, { sourceFileNames: loaded.map(([, entry]) => entry.submissionId), now });

    mkdirSync(outputDir, { recursive: true });
    // meta.json marks a complete run: remove it until all modules are written.
    rmSync(join(outputDir, "meta.json"), { force: true });
    for (const [field, fileName] of Object.entries(OUTPUT_FILES)) {
      writeFileSync(join(outputDir, fileName), bundle[field as keyof typeof OUTPUT_FILES]);
    }
    writeLookups(finalizeLookups(lookupAcc), outputDir);

    const meta: ExtractMeta = {
      sessionCount: acc.sessionCount,
      submissionCount: loaded.length,
      exportCount: loaded.reduce((sum, [, entry]) => sum + entry.exportCount, 0),
      skippedFiles: entries.flatMap(([name, entry]) => (entry.error === undefined ? [] : [{ name, error: entry.error }])),
      partialFiles: loaded.flatMap(([name, entry]) => (entry.droppedExports.length === 0 ? [] : [{ name, droppedExports: entry.droppedExports }])),
      duplicateSessionsSkipped: duplicateCopies + loaded.reduce((sum, [, entry]) => sum + entry.duplicates, 0),
      generatedAt: now.toISOString(),
    };
    writeFileSync(join(outputDir, "meta.json"), JSON.stringify(meta, null, 2));
    return meta;
  } finally {
    acc.observations.dispose();
  }
}

/**
 * Observes every file on the `pool` of workers (on the main thread when it is empty) and hands
 * the results to `fold` in the order of `files`. At most a few results wait for an earlier one,
 * so memory use stays bounded. The workers learn the session keys `fold` returns; `usedKeys`
 * must already hold those of earlier calls.
 */
async function observeInOrder(
  files: string[],
  maxIds: ObserveRequest["maxIds"],
  pool: Worker[],
  usedKeys: ReadonlySet<string>,
  fold: (file: string, submission: ObservedSubmission, index: number) => Set<string> | void,
): Promise<void> {
  if (pool.length === 0) {
    files.forEach((file, index) => fold(file, observeSubmission({ path: file, maxIds }, usedKeys), index));
    return;
  }

  const results = new Map<number, ObservedSubmission>();
  let next = 0;
  let folded = 0;
  let stopped = false;
  let onError: ((e: Error) => void) | undefined;
  let onExit: ((code: number) => void) | undefined;
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
      onError = (e: Error) => {
        stopped = true;
        rejectAll(e);
      };
      // A worker that exits without an error would otherwise leave its file unfolded forever.
      onExit = (code: number) => {
        stopped = true;
        rejectAll(new Error(`An observe worker exited early (code ${code})`));
      };
      for (const worker of pool) {
        worker.on("error", onError);
        worker.on("exit", onExit);
      }
      for (const worker of pool) dispatch(worker);
      if (files.length === 0) resolveAll();
    });
  } finally {
    for (const worker of pool) {
      if (onError) worker.off("error", onError);
      if (onExit) worker.off("exit", onExit);
    }
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
