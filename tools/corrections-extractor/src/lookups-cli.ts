import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import { extractLookups, type LookupMeta } from "./extract-lookups";

const toolDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const traceDir = resolve(toolDir, "../../Traces");
const outputDir = resolve(toolDir, "output");

let meta: LookupMeta;
try {
  meta = extractLookups(traceDir, outputDir);
} catch (e) {
  console.error(e instanceof Error ? e.message : String(e));
  process.exit(1);
}

console.log(
  `Extracted lookups from ${meta.sessionCount} session(s) in ${meta.loadedFileNames.length} trace file(s) into ${outputDir}`,
);
for (const skipped of meta.skippedFiles) {
  console.warn(`Skipped ${skipped.name}: ${skipped.error}`);
}
