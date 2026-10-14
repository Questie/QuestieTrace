// Runs the lookup extraction over every trace file in a directory and writes
// one lookup<Entity>/<locale>.lua module per entity and locale.

import { mkdirSync, rmSync, writeFileSync } from "fs";
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
  const bundle = finalizeLookups(acc);

  for (const [dirName, files] of Object.entries(bundle)) {
    const dir = join(outputDir, dirName);
    // Drop stale locales from a previous run.
    rmSync(dir, { recursive: true, force: true });
    mkdirSync(dir, { recursive: true });
    for (const [locale, content] of Object.entries(files)) {
      writeFileSync(join(dir, `${locale}.lua`), content);
    }
  }

  return { ...result, sessionCount: acc.sessionCount };
}
