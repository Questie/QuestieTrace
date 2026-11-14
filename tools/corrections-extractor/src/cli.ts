import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import { extractCorrections, workersFromEnv, type ExtractMeta } from "./extract-corrections";
import { sampledSource, samplePercentFromEnv, sourceFromEnv } from "./store/source";
import { formatDuration } from "./timing";

const toolDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
/** TRACE_STORE_DIR, else trace-session-store next to the QuestieTrace checkout. */
const fullStoreDir = process.env.TRACE_STORE_DIR ? resolve(process.env.TRACE_STORE_DIR) : resolve(toolDir, "../../../trace-session-store");

const startedAt = performance.now();
let meta: ExtractMeta;
let outputDir = resolve(toolDir, "output");
try {
  // A sample gets its own store and output, so it never touches the full ones.
  const sample = samplePercentFromEnv();
  let source = sourceFromEnv();
  let storeDir = fullStoreDir;
  if (sample !== undefined) {
    source = sampledSource(source, sample);
    storeDir = resolve(fullStoreDir, `sample-${sample}`);
    outputDir = resolve(outputDir, `sample-${sample}`);
  }
  meta = await extractCorrections(source, storeDir, outputDir, { workers: workersFromEnv() });
} catch (e) {
  console.error(e instanceof Error ? e.message : String(e));
  process.exit(1);
}

console.log(
  `Extracted ${meta.sessionCount} session(s) from ${meta.submissionCount} submission(s) into ${outputDir} in ${formatDuration(performance.now() - startedAt)}`,
);
for (const skipped of meta.skippedFiles) {
  console.warn(`Skipped ${skipped.name}: ${skipped.error}`);
}
for (const partial of meta.partialFiles) {
  console.warn(`Partly decoded ${partial.name}: ${partial.droppedExports.join("; ")}`);
}
if (meta.duplicateSessionsSkipped > 0) {
  // Exports contain every saved session, so later submissions repeat earlier sessions.
  console.log(`Ignored ${meta.duplicateSessionsSkipped} older copies of sessions that later submissions exported again (the newest copy is used)`);
}
