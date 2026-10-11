// `npm run relations -- signal:breadcrumbs`: optional lead-in quests (breadcrumbs) from episodes.
// Streams episodes twice (links, then observations) so they are never all in memory.

import { resolve } from "path";
import { isForeverInterface } from "../../core/eligibility";
import { loadCatalog, readEpisodes, writeCandidates, writeJsonAtomic } from "../../core/io";
import { paths } from "../../core/paths";
import { DEFAULT_PARAMS, LinkCollector } from "./links";
import { judgeTally, MIN_COVERAGE_CHARACTERS, Tally } from "./infer";
import { observeEpisode } from "./observe";

export async function run(_args: string[]): Promise<void> {
  const catalog = loadCatalog();
  const params = DEFAULT_PARAMS;

  const collector = new LinkCollector(catalog, params);
  let inputCount = 0;
  for await (const episode of readEpisodes()) {
    if (!isForeverInterface(episode.interfaceVersion)) continue;
    collector.add(episode);
    inputCount++;
  }
  const links = collector.finish();

  const tally = new Tally();
  for await (const episode of readEpisodes()) {
    if (!isForeverInterface(episode.interfaceVersion)) continue;
    tally.add(episode.characterKey, observeEpisode(episode, links, catalog, params));
  }
  const result = judgeTally(tally, catalog);

  const path = writeCandidates({
    signal: "breadcrumbs",
    generatedAt: new Date().toISOString(),
    inputCount,
    params: { ...params, minCoverageCharacters: MIN_COVERAGE_CHARACTERS },
    coveredQuestIds: result.coveredQuestIds,
    candidates: result.candidates,
  });

  // Every judged pair with its per-kind character counts, for review and calibration.
  const review = result.judgements
    .sort((a, b) => b.score - a.score)
    .map(({ breadcrumb, target, score, support, contradict, counts }) => ({
      breadcrumb,
      target,
      breadcrumbName: catalog.quests[breadcrumb]?.name,
      targetName: catalog.quests[target]?.name,
      score,
      support,
      contradict,
      counts,
    }));
  writeJsonAtomic(resolve(paths.reportsDir, "breadcrumbs-pairs.json"), { linkedPairs: tally.pairs.size, judged: review.length, pairs: review });

  console.log(
    `breadcrumbs: ${inputCount} Forever episodes, ${tally.pairs.size} observed linked pairs, ${result.judgements.length} optional+dependent, ` +
      `${result.candidates.length / 2} edges, ${result.coveredQuestIds.length} covered quests -> ${path}`,
  );
}
