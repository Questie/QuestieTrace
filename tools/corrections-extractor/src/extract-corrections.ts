// Runs the corrections extraction and writes the forever*Traces.lua modules plus a meta.json
// summary. The input is either
//
// - "traces": every decoded trace file (*.lua) in a directory, Traces/ by default, or
// - "trace-data": every submission in a trace-data checkout, decoded natively. Exports are
//   cumulative and include the in-progress session, so one session shows up in many
//   submissions, growing as play continues. Submissions are read newest first and only the
//   first copy of each session met (the newest, so the most complete: see
//   core/trace-data/session-key.ts) is folded in.
//
// Input is loaded and folded in one file or submission at a time and then dropped, so memory
// use is bounded by the collected observations rather than by the total size of the input.

import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "fs";
import { join, relative, resolve } from "path";
import { loadTraceFile } from "./core/loader";
import { sessionKey } from "./core/trace-data/session-key";
import { listSubmissionFiles, readSubmission } from "./core/trace-data/submissions";
import type { SessionRecord } from "./core/types";
import { createAccumulator, finalize, foldSessions, type ExtractAccumulator } from "./extract";

const OUTPUT_FILES = {
  npcFixes: "foreverNpcTraces.lua",
  questFixes: "foreverQuestTraces.lua",
  itemFixes: "foreverItemTraces.lua",
  objectFixes: "foreverObjectTraces.lua",
} as const;

export type ExtractSource = "traces" | "trace-data";

export interface ExtractMeta {
  sessionCount: number;
  /** Trace files folded in: Traces/ files, or the exports decoded from trace-data submissions. */
  fileCount: number;
  /** Traces/ files, or submission files relative to submissions/, that failed to load. */
  skippedFiles: { name: string; error: string }[];
  generatedAt: string;
  source: ExtractSource;
  /** Submissions folded in; 0 for "traces". */
  submissionCount: number;
  /** Session copies skipped because a newer copy was already folded in; 0 for "traces". */
  duplicateSessionsSkipped: number;
}

export interface ExtractArgs {
  source: ExtractSource;
  dir: string;
}

/** Parses `[--trace-data [dir]]`: Traces/ without the flag, else the given or default trace-data checkout. */
export function parseExtractArgs(args: string[], defaults: { traceDir: string; traceDataDir: string }): ExtractArgs {
  if (args.length === 0) return { source: "traces", dir: defaults.traceDir };
  const [flag, dir, ...rest] = args;
  if (flag !== "--trace-data" || rest.length > 0 || dir === "" || dir?.startsWith("-")) {
    throw new Error(`Usage: npm run extract [-- --trace-data [dir]] (got: ${args.join(" ")})`);
  }
  return { source: "trace-data", dir: dir === undefined ? defaults.traceDataDir : resolve(dir) };
}

/** Extracts from every .lua trace file in `traceDir`. */
export function extractCorrections(
  traceDir: string,
  outputDir: string,
  log: (message: string) => void = console.log,
): ExtractMeta {
  if (!existsSync(traceDir)) {
    throw new Error(`Trace directory not found: ${traceDir}`);
  }
  const fileNames = readdirSync(traceDir)
    .filter((f) => f.endsWith(".lua"))
    .sort();
  if (fileNames.length === 0) {
    throw new Error(`No .lua trace files found in ${traceDir}`);
  }

  const acc = createAccumulator();
  const loadedFileNames: string[] = [];
  const skippedFiles: ExtractMeta["skippedFiles"] = [];

  fileNames.forEach((fileName, index) => {
    let error: string | undefined;
    try {
      error = foldAllOrNothing(acc, loadTraceFile(join(traceDir, fileName)).sessions);
    } catch (e) {
      // Load errors of these local files stay verbatim: they point at the broken Lua.
      error = String(e);
    }
    if (error) skippedFiles.push({ name: fileName, error });
    else loadedFileNames.push(fileName);
    log(`[${index + 1}/${fileNames.length}] ${fileName}`);
  });

  return writeOutputs(acc, outputDir, loadedFileNames, {
    fileCount: loadedFileNames.length,
    skippedFiles,
    source: "traces",
    submissionCount: 0,
    duplicateSessionsSkipped: 0,
  });
}

/** Extracts from every submission in a trace-data checkout, folding each session's newest copy only. */
export function extractCorrectionsFromTraceData(
  traceDataDir: string,
  outputDir: string,
  log: (message: string) => void = console.log,
): ExtractMeta {
  // Newest first: the first copy of a session met is then its most complete one.
  const files = listSubmissionFiles(traceDataDir).reverse();
  const submissionsDir = resolve(traceDataDir, "submissions");

  const acc = createAccumulator();
  const foldedKeys = new Set<string>();
  const foldedSubmissionIds: string[] = [];
  const skippedFiles: ExtractMeta["skippedFiles"] = [];
  let fileCount = 0;
  let duplicateSessionsSkipped = 0;

  files.forEach((file, index) => {
    const name = relative(submissionsDir, file.path);
    const submission = readSubmission(file.path);
    if (submission.error) {
      skippedFiles.push({ name, error: submission.error });
    } else {
      // Keys count as folded only once the whole submission is: when it fails, the older copies
      // of its sessions are folded in instead.
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
      const error = foldAllOrNothing(acc, sessions);
      if (error) {
        skippedFiles.push({ name, error });
      } else {
        for (const key of keys) foldedKeys.add(key);
        duplicateSessionsSkipped += duplicates;
        foldedSubmissionIds.push(submission.submissionId);
        fileCount += submission.traces.length;
      }
    }
    if ((index + 1) % 250 === 0 || index + 1 === files.length) log(`[${index + 1}/${files.length}] submissions`);
  });

  return writeOutputs(acc, outputDir, foldedSubmissionIds, {
    fileCount,
    skippedFiles,
    source: "trace-data",
    submissionCount: foldedSubmissionIds.length,
    duplicateSessionsSkipped,
  });
}

/**
 * Folds `sessions` into `acc` all or nothing. An observer throwing part-way would leave the
 * sessions before it folded in, so they go through a throwaway accumulator first: folding is
 * cheap next to loading (22s of a 5-minute trace-data run). Returns the error to report: only
 * the error's name, since its message can quote trace data.
 */
function foldAllOrNothing(acc: ExtractAccumulator, sessions: SessionRecord[]): string | undefined {
  try {
    foldSessions(createAccumulator(), sessions);
  } catch (e) {
    return `folding the sessions failed (${e instanceof Error ? e.name : typeof e})`;
  }
  foldSessions(acc, sessions);
  return undefined;
}

function writeOutputs(
  acc: ExtractAccumulator,
  outputDir: string,
  sourceFileNames: string[],
  summary: Omit<ExtractMeta, "sessionCount" | "generatedAt">,
): ExtractMeta {
  const now = new Date();
  const bundle = finalize(acc, { sourceFileNames, now });

  mkdirSync(outputDir, { recursive: true });
  // meta.json marks a complete run: remove it until all modules are written.
  rmSync(join(outputDir, "meta.json"), { force: true });
  for (const [field, fileName] of Object.entries(OUTPUT_FILES)) {
    writeFileSync(join(outputDir, fileName), bundle[field as keyof typeof OUTPUT_FILES]);
  }

  const meta: ExtractMeta = {
    sessionCount: acc.sessionCount,
    fileCount: summary.fileCount,
    skippedFiles: summary.skippedFiles,
    generatedAt: now.toISOString(),
    source: summary.source,
    submissionCount: summary.submissionCount,
    duplicateSessionsSkipped: summary.duplicateSessionsSkipped,
  };
  writeFileSync(join(outputDir, "meta.json"), JSON.stringify(meta, null, 2));
  return meta;
}
