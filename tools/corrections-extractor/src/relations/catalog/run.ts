// `npm run relations -- catalog`: QuestieDB -> catalog.json + groundtruth.json (contract:
// src/relations/README.md). QuestieDB's own Lua tooling materializes Forever's quests
// (./forever-quests.lua); ./build.ts shapes them into the core contracts.

import { resolve } from "path";
import { writeJsonAtomic } from "../core/io";
import { paths } from "../core/paths";
import { buildCatalog, buildGroundTruth, EXPORTED_FIELDS } from "./build";
import { materializeForeverQuests } from "./materialize";
import { formatSummary, summarize } from "./summary";

export async function run(_args: string[]): Promise<void> {
  const materialized = materializeForeverQuests(paths.questieDbDir, EXPORTED_FIELDS);
  const generatedAt = new Date().toISOString();
  const catalog = buildCatalog(materialized, generatedAt);
  const truth = buildGroundTruth(materialized, generatedAt);
  const summary = summarize(materialized, catalog, truth);

  writeJsonAtomic(paths.catalog, catalog);
  writeJsonAtomic(paths.groundTruth, truth);
  writeJsonAtomic(resolve(paths.reportsDir, "catalog.json"), summary);
  console.log(formatSummary(summary));
  console.log(`wrote ${paths.catalog}, ${paths.groundTruth}`);
}
