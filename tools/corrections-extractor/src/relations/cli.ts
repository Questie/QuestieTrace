// `npm run relations -- <command> [args]`: one entry point for every area of the
// quest-relationship pipeline (see README.md next to this file).
//
// Each area owns a `run.ts` exporting `run(args)`. Modules are imported lazily so
// one area's runtime dependencies never load for another area's command.

import { formatScoreReport, scoreCandidates } from "./core/score";
import { loadCandidates, loadGroundTruth, writeJsonAtomic } from "./core/io";
import { paths } from "./core/paths";
import { resolve } from "path";

type Runner = (args: string[]) => Promise<void>;

const COMMANDS: Record<string, { describe: string; load: () => Promise<Runner> }> = {
  ingest: { describe: "trace-data submissions -> episodes.jsonl", load: async () => (await import("./ingest/run")).run },
  catalog: { describe: "QuestieDB -> catalog.json + groundtruth.json", load: async () => (await import("./catalog/run")).run },
  "source:wowhead": {
    describe: "Wowhead scrape -> candidates/wowhead-*.json",
    load: async () => (await import("./sources/wowhead/run")).run,
  },
  "signal:handoff": { describe: "turn-in -> immediate next offer chains", load: async () => (await import("./signals/handoff/run")).run },
  "signal:offer-set": {
    describe: "completed sets at offer time -> prerequisites",
    load: async () => (await import("./signals/offer-set/run")).run,
  },
  "signal:absence": {
    describe: "complete offer lists without a quest -> blockers/exclusivity",
    load: async () => (await import("./signals/absence/run")).run,
  },
  "signal:cooccurrence": {
    describe: "completion histories across characters -> implication/exclusion",
    load: async () => (await import("./signals/cooccurrence/run")).run,
  },
  "signal:breadcrumbs": { describe: "optional lead-in quests -> breadcrumbs", load: async () => (await import("./signals/breadcrumbs/run")).run },
  combine: { describe: "all candidates -> reviewed relations + Questie output", load: async () => (await import("./combine/run")).run },
  score: { describe: "score <signal> [--min-score n]: precision/recall vs groundtruth.json", load: async () => runScore },
  pipeline: { describe: "every step in order: ingest, catalog, sources, signals, combine", load: async () => runPipeline },
};

/** Commands `pipeline` runs, in dependency order. */
const PIPELINE_STEPS = [
  "ingest",
  "catalog",
  "source:wowhead",
  "signal:handoff",
  "signal:offer-set",
  "signal:absence",
  "signal:cooccurrence",
  "signal:breadcrumbs",
  "combine",
];

async function runPipeline(): Promise<void> {
  for (const step of PIPELINE_STEPS) {
    const started = Date.now();
    console.log(`\n== ${step}`);
    await (await COMMANDS[step].load())([]);
    console.log(`== ${step} done in ${((Date.now() - started) / 1000).toFixed(1)}s`);
  }
}

async function runScore(args: string[]): Promise<void> {
  const signal = args[0];
  if (!signal) throw new Error("usage: score <signal> [--min-score n]");
  const minScoreIndex = args.indexOf("--min-score");
  const minScore = minScoreIndex >= 0 ? Number(args[minScoreIndex + 1]) : 0;
  if (!Number.isFinite(minScore)) throw new Error(`--min-score needs a number, got "${args[minScoreIndex + 1]}"`);
  const report = scoreCandidates(loadCandidates(signal), loadGroundTruth(), { minScore });
  writeJsonAtomic(resolve(paths.reportsDir, `score-${signal}.json`), report);
  console.log(formatScoreReport(report));
}

function printUsage(): void {
  console.log("usage: npm run relations -- <command> [args]\n");
  for (const [name, command] of Object.entries(COMMANDS)) console.log(`  ${name.padEnd(20)} ${command.describe}`);
}

const [commandName, ...args] = process.argv.slice(2);
const command = commandName ? COMMANDS[commandName] : undefined;
if (!command) {
  printUsage();
  process.exit(commandName ? 1 : 0);
}

try {
  await (await command.load())(args);
} catch (e) {
  console.error(e instanceof Error ? (e.stack ?? e.message) : String(e));
  process.exit(1);
}
