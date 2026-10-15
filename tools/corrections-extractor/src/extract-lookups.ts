// Runs the lookup extraction over every trace file in a directory and writes
// one lookup<Entity>/<locale>.lua module per entity and locale.

import { mkdirSync, renameSync, rmSync, writeFileSync } from "fs";
import { join } from "path";
import { foldTraceFiles, type FoldResult } from "./extract-corrections";
import { createLookupAccumulator, finalizeLookups, foldLookupSessions } from "./extract/lookups";

export interface LookupMeta extends FoldResult {
  sessionCount: number;
}

export function extractLookups(
  traceDir: string,
  outputDir: string,
  log: (message: string) => void = console.log,
): LookupMeta {
  const acc = createLookupAccumulator();
  const result = foldTraceFiles(traceDir, (sessions) => foldLookupSessions(acc, sessions), log);
  if (result.loadedFileNames.length === 0) {
    throw new Error(`No trace file in ${traceDir} could be loaded; keeping the previous lookups`);
  }
  const bundle = finalizeLookups(acc);

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

  return { ...result, sessionCount: acc.sessionCount };
}
