// Runs the corrections extraction over every trace file in a directory and
// writes the forever*Traces.lua modules plus a meta.json summary.
//
// Trace files are loaded and folded in one at a time and then dropped, so
// memory use is bounded by the collected observations rather than by the
// total size of all trace files.

import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "fs";
import { join } from "path";
import { loadTraceFile } from "./core/loader";
import type { SessionRecord } from "./core/types";
import { createAccumulator, finalize, foldSessions } from './extract';

const OUTPUT_FILES = {
  npcFixes: "foreverNpcTraces.lua",
  questFixes: "foreverQuestTraces.lua",
  itemFixes: "foreverItemTraces.lua",
  objectFixes: "foreverObjectTraces.lua",
} as const;

export interface ExtractMeta {
  sessionCount: number;
  fileCount: number;
  skippedFiles: { name: string; error: string }[];
  duplicateSessions: DuplicateSession[];
  generatedAt: string;
}

/** A session that was already loaded from another trace file, e.g. an in-progress session exported again once finished. */
export interface DuplicateSession {
  name: string;
  firstName: string;
  startedAtPrecise: number;
}

export interface FoldResult {
  loadedFileNames: string[];
  skippedFiles: ExtractMeta["skippedFiles"];
  duplicateSessions: DuplicateSession[];
}

/**
 * Loads every trace file in `traceDir` one at a time and passes its sessions to `fold`.
 * `fold` must be all-or-nothing, so a skipped file contributes no data.
 *
 * A session that was already loaded from an earlier file (same `startedAtPrecise`) is
 * skipped and reported, so it is not counted twice.
 */
export function foldTraceFiles(
  traceDir: string,
  fold: (sessions: SessionRecord[]) => void,
  log: (message: string) => void,
): FoldResult {
  if (!existsSync(traceDir)) {
    throw new Error(`Trace directory not found: ${traceDir}`);
  }
  const fileNames = readdirSync(traceDir)
    .filter((f) => f.endsWith(".lua"))
    .sort();
  if (fileNames.length === 0) {
    throw new Error(`No .lua trace files found in ${traceDir}`);
  }

  const loadedFileNames: string[] = [];
  const skippedFiles: ExtractMeta["skippedFiles"] = [];
  const duplicateSessions: DuplicateSession[] = [];
  const seenSessions = new Map<number, string>();

  fileNames.forEach((fileName, index) => {
    let sessions: SessionRecord[] | undefined;
    try {
      sessions = loadTraceFile(join(traceDir, fileName)).sessions;
    } catch (e) {
      skippedFiles.push({ name: fileName, error: `Failed to load: ${String(e)}` });
    }
    if (sessions) {
      const unique: SessionRecord[] = [];
      for (const session of sessions) {
        const firstName = seenSessions.get(session.startedAtPrecise);
        if (firstName === undefined) {
          unique.push(session);
        } else {
          duplicateSessions.push({ name: fileName, firstName, startedAtPrecise: session.startedAtPrecise });
        }
      }
      try {
        fold(unique);
        for (const session of unique) seenSessions.set(session.startedAtPrecise, fileName);
        loadedFileNames.push(fileName);
      } catch (e) {
        skippedFiles.push({ name: fileName, error: `Failed to extract: ${String(e)}` });
      }
    }
    log(`[${index + 1}/${fileNames.length}] ${fileName}`);
  });

  return { loadedFileNames, skippedFiles, duplicateSessions };
}

export function extractCorrections(
  traceDir: string,
  outputDir: string,
  log: (message: string) => void = console.log,
): ExtractMeta {
  const acc = createAccumulator();
  const { loadedFileNames, skippedFiles, duplicateSessions } = foldTraceFiles(traceDir, (sessions) => foldSessions(acc, sessions), log);

  const now = new Date();
  const bundle = finalize(acc, { sourceFileNames: loadedFileNames, now });

  mkdirSync(outputDir, { recursive: true });
  // meta.json marks a complete run: remove it until all modules are written.
  rmSync(join(outputDir, "meta.json"), { force: true });
  for (const [field, fileName] of Object.entries(OUTPUT_FILES)) {
    writeFileSync(join(outputDir, fileName), bundle[field as keyof typeof OUTPUT_FILES]);
  }

  const meta: ExtractMeta = {
    sessionCount: acc.sessionCount,
    fileCount: loadedFileNames.length,
    skippedFiles,
    duplicateSessions,
    generatedAt: now.toISOString(),
  };
  writeFileSync(join(outputDir, "meta.json"), JSON.stringify(meta, null, 2));
  return meta;
}
