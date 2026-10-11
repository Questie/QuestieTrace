// The absence signal end to end, over any episode source that can be read twice.

import { isForeverInterface } from "../../core/eligibility";
import type { QuestCatalog, QuestEpisode, RelationCandidate } from "../../core/types";
import { BlockedCollector } from "./blocked";
import { classify } from "./classify";
import { OfferIndexBuilder, type OfferIndex } from "./offers";
import type { AbsenceParams } from "./params";

export interface AbsenceResult {
  index: OfferIndex;
  blocked: BlockedCollector;
  candidates: RelationCandidate[];
  /** Quests with at least one blocked moment that passed gating. */
  coveredQuestIds: number[];
}

type EpisodeSource = () => Iterable<QuestEpisode> | AsyncIterable<QuestEpisode>;

export async function analyzeAbsence(readEpisodes: EpisodeSource, catalog: QuestCatalog, params: AbsenceParams): Promise<AbsenceResult> {
  const offers = new OfferIndexBuilder(params);
  for await (const episode of readEpisodes()) if (isForeverInterface(episode.interfaceVersion)) offers.add(episode);
  const index = offers.build((questId) => catalog.quests[questId]?.name);

  const blocked = new BlockedCollector(index, catalog, params);
  for await (const episode of readEpisodes()) if (isForeverInterface(episode.interfaceVersion)) blocked.add(episode);

  const candidates = classify(blocked.quests, index, blocked.locality, catalog, params);
  const coveredQuestIds = [...blocked.quests.keys()].sort((a, b) => a - b);
  return { index, blocked, candidates, coveredQuestIds };
}
