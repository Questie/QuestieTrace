// `npm run extract [-- --trace-data [dir]]`: Traces/ by default, else a trace-data checkout
// (TRACE_DATA_DIR or the repo's trace-data/ when no dir is given).

import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import { defaultTraceDataDir } from "./core/trace-data/submissions";
import { extractCorrections, extractCorrectionsFromTraceData, parseExtractArgs, type ExtractMeta } from "./extract-corrections";

const toolDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputDir = resolve(toolDir, "output");

let meta: ExtractMeta;
try {
  const { source, dir } = parseExtractArgs(process.argv.slice(2), {
    traceDir: resolve(toolDir, "../../Traces"),
    traceDataDir: defaultTraceDataDir(),
  });
  meta = source === "trace-data" ? extractCorrectionsFromTraceData(dir, outputDir) : extractCorrections(dir, outputDir);
} catch (e) {
  console.error(e instanceof Error ? e.message : String(e));
  process.exit(1);
}

if (meta.source === "trace-data") {
  console.log(
    `Extracted ${meta.sessionCount} session(s) from ${meta.fileCount} export(s) in ${meta.submissionCount} submission(s) ` +
      `(${meta.duplicateSessionsSkipped} older session copies skipped) into ${outputDir}`,
  );
} else {
  console.log(`Extracted ${meta.sessionCount} session(s) from ${meta.fileCount} trace file(s) into ${outputDir}`);
}
for (const skipped of meta.skippedFiles) {
  console.warn(`Skipped ${skipped.name}: ${skipped.error}`);
}
