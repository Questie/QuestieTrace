import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import { defaultTraceDataDir } from "./core/trace-data/submissions";
import { extractCorrections, type ExtractMeta } from "./extract-corrections";

const toolDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputDir = resolve(toolDir, "output");

const startedAt = performance.now();
let meta: ExtractMeta;
try {
  meta = await extractCorrections(defaultTraceDataDir(), outputDir);
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

function formatDuration(ms: number): string {
  const seconds = Math.round(ms / 1000);
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}
