import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import { extractCorrections, type ExtractMeta } from "./extract-corrections";

const toolDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const traceDir = resolve(toolDir, "../../Traces");
const outputDir = resolve(toolDir, "output");

let meta: ExtractMeta;
try {
  meta = extractCorrections(traceDir, outputDir);
} catch (e) {
  console.error(e instanceof Error ? e.message : String(e));
  process.exit(1);
}

console.log(
  `Extracted ${meta.sessionCount} session(s) from ${meta.fileCount} trace file(s) into ${outputDir}`,
);
for (const skipped of meta.skippedFiles) {
  console.warn(`Skipped ${skipped.name}: ${skipped.error}`);
}
if (meta.duplicateSessions.length > 0) {
  const fileCount = new Set(meta.duplicateSessions.map((duplicate) => duplicate.name)).size;
  console.warn(
    `Warning: skipped ${meta.duplicateSessions.length} duplicate session(s) in ${fileCount} trace file(s), see duplicateSessions in meta.json`,
  );
}
