// Runs the corrections extraction over every trace file in a directory and
// writes the forever*Traces.lua modules plus a meta.json summary.
//
// Trace files are loaded and folded in one at a time and then dropped, so
// memory use is bounded by the collected observations rather than by the
// total size of all trace files.

import { mkdirSync, readdirSync, writeFileSync } from "fs";
import { join } from "path";
import { loadTraceFile } from "./core/loader";
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
  generatedAt: string;
}

export function extractCorrections(
  traceDir: string,
  outputDir: string,
  log: (message: string) => void = console.log,
): ExtractMeta {
  const fileNames = readdirSync(traceDir)
    .filter((f) => f.endsWith(".lua"))
    .sort();

  const acc = createAccumulator();
  const loadedFileNames: string[] = [];
  const skippedFiles: ExtractMeta["skippedFiles"] = [];

  fileNames.forEach((fileName, index) => {
    try {
      foldSessions(acc, loadTraceFile(join(traceDir, fileName)).sessions);
      loadedFileNames.push(fileName);
    } catch (e) {
      skippedFiles.push({ name: fileName, error: String(e) });
    }
    log(`[${index + 1}/${fileNames.length}] ${fileName}`);
  });

  const now = new Date();
  const bundle = finalize(acc, { sourceFileNames: loadedFileNames, now });

  mkdirSync(outputDir, { recursive: true });
  for (const [field, fileName] of Object.entries(OUTPUT_FILES)) {
    writeFileSync(join(outputDir, fileName), bundle[field as keyof typeof OUTPUT_FILES]);
  }

  const meta: ExtractMeta = {
    sessionCount: acc.sessionCount,
    fileCount: loadedFileNames.length,
    skippedFiles,
    generatedAt: now.toISOString(),
  };
  writeFileSync(join(outputDir, "meta.json"), JSON.stringify(meta, null, 2));
  return meta;
}
