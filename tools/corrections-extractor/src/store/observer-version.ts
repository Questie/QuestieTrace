// Identity of the code that produces observations. Stored observations were made by one
// version; when the code changes, they are dropped and every submission is observed again.
//
// Hashes the sources the observers run (and the MAX_IDS they filter by, in extract/observe.ts).
// Merge functions live in extract/merge/ and are not hashed, like index.ts, lookups.ts, emit/
// and schema/: changing them only re-merges.
// How the store writes observations is versioned by hand: FORMAT_VERSION in observed-store.ts.

import { createHash } from "crypto";
import { readdirSync, readFileSync, statSync } from "fs";
import { dirname, join, relative, resolve } from "path";
import { fileURLToPath } from "url";

const srcDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export const OBSERVER_SOURCES = [
  "core",
  "extract/observers",
  "extract/resources",
  "extract/correlate.ts",
  "extract/guid.ts",
  "extract/itemLink.ts",
  "extract/observation.ts",
  "extract/observe.ts",
  "extract/observe-lookups.ts",
  "extract/probes.ts",
  "observe-submission.ts",
  "observe-worker.ts",
];

function isSource(path: string): boolean {
  return !path.endsWith(".test.ts") && !path.includes("__fixtures__") && !path.endsWith("fixtures.ts");
}

function files(path: string): string[] {
  if (!statSync(path).isDirectory()) return isSource(path) ? [path] : [];
  return readdirSync(path).flatMap((name) => files(join(path, name)));
}

export function observerVersion(root = srcDir, sources = OBSERVER_SOURCES): string {
  const hash = createHash("sha256");
  for (const file of sources.flatMap((source) => files(join(root, source))).sort()) {
    hash.update(relative(root, file).split("\\").join("/")).update("\0").update(readFileSync(file)).update("\0");
  }
  return hash.digest("hex").slice(0, 16);
}
