import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import { extractCorrections } from "./extract-corrections";

const toolDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const traceDir = resolve(toolDir, "../../Traces");
const outputDir = resolve(toolDir, "output");

const meta = extractCorrections(traceDir, outputDir);

console.log(
  `Extracted ${meta.sessionCount} session(s) from ${meta.fileCount} trace file(s) into ${outputDir}`,
);
for (const skipped of meta.skippedFiles) {
  console.warn(`Skipped ${skipped.name}: ${skipped.error}`);
}
