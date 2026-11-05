// Runs the corrections and lookup extraction over every submission in a trace-data checkout
// and writes the forever*Traces.lua modules, one lookup<Entity>/<locale>.lua module per
// entity and locale, plus a meta.json summary.
//
// Submissions are decoded and folded in one at a time and then dropped, so memory use is
// bounded by the collected observations rather than by the total size of the input.
//
// Exports are cumulative and include the in-progress session, so one session shows up in
// many submissions, growing as play continues. Submissions are read newest first and only
// the first copy of each session met (the newest, so the most complete: see
// core/trace-data/session-key.ts) is folded in.

import { mkdirSync, renameSync, rmSync, writeFileSync } from "fs";
import { join, relative, resolve } from "path";
import { sessionKey } from "./core/trace-data/session-key";
import { listSubmissionFiles, readSubmission } from "./core/trace-data/submissions";
import type { SessionRecord } from "./core/types";
import { createAccumulator, finalize, foldSessions } from "./extract";
import { createLookupAccumulator, finalizeLookups, foldLookupSessions, type LookupBundle } from "./extract/lookups";

const OUTPUT_FILES = {
  npcFixes: "foreverNpcTraces.lua",
  questFixes: "foreverQuestTraces.lua",
  itemFixes: "foreverItemTraces.lua",
  objectFixes: "foreverObjectTraces.lua",
} as const;

export interface ExtractMeta {
  sessionCount: number;
  /** Submissions folded in. */
  submissionCount: number;
  /** Exports decoded from the folded submissions; a submission may hold several. */
  exportCount: number;
  /** Submission files, relative to submissions/, that failed to load. */
  skippedFiles: { name: string; error: string }[];
  /** Session copies skipped because a newer copy was already folded in. */
  duplicateSessionsSkipped: number;
  generatedAt: string;
}

export function extractCorrections(
  traceDataDir: string,
  outputDir: string,
  log: (message: string) => void = console.log,
): ExtractMeta {
  // Newest first: the first copy of a session met is then its most complete one.
  const files = listSubmissionFiles(traceDataDir).reverse();
  const submissionsDir = resolve(traceDataDir, "submissions");

  const acc = createAccumulator();
  const lookupAcc = createLookupAccumulator();
  const foldedKeys = new Set<string>();
  const foldedSubmissionIds: string[] = [];
  const skippedFiles: ExtractMeta["skippedFiles"] = [];
  let exportCount = 0;
  let duplicateSessionsSkipped = 0;

  files.forEach((file, index) => {
    const name = relative(submissionsDir, file.path);
    const submission = readSubmission(file.path);
    if (submission.error) {
      skippedFiles.push({ name, error: submission.error });
    } else {
      const keys = new Set<string>();
      const sessions: SessionRecord[] = [];
      let duplicates = 0;
      for (const session of submission.traces.flatMap((trace) => trace.sessions)) {
        // A session without a key cannot be matched with its copies, so every copy is folded in.
        const key = sessionKey(session);
        if (key !== undefined && (foldedKeys.has(key) || keys.has(key))) {
          duplicates++;
        } else {
          if (key !== undefined) keys.add(key);
          sessions.push(session);
        }
      }
      // Observer errors abort the run, keeping the previous output: a submission folded in
      // only part way could not be taken out of the accumulators again.
      foldSessions(acc, sessions);
      foldLookupSessions(lookupAcc, sessions);
      for (const key of keys) foldedKeys.add(key);
      duplicateSessionsSkipped += duplicates;
      foldedSubmissionIds.push(submission.submissionId);
      exportCount += submission.traces.length;
    }
    if ((index + 1) % 250 === 0 || index + 1 === files.length) log(`[${index + 1}/${files.length}] submissions`);
  });

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
    duplicateSessionsSkipped,
    generatedAt: now.toISOString(),
  };
  writeFileSync(join(outputDir, "meta.json"), JSON.stringify(meta, null, 2));
  return meta;
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
        writeFileSync(join(dir, `${locale}.lua`), content);
      }
    }
    for (const dirName of Object.keys(bundle)) {
      // Replacing the whole directory also drops stale locales from a previous run.
      rmSync(join(outputDir, dirName), { recursive: true, force: true });
      renameSync(join(stagingDir, dirName), join(outputDir, dirName));
    }
  } finally {
    rmSync(stagingDir, { recursive: true, force: true });
  }
}
