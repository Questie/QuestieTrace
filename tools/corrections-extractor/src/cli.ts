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
