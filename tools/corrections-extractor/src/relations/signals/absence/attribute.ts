// Who takes the blame for a quest's blocked moments: EM over a mixture of causes.
//
// Each blocked moment has one cause: one of its suspects, or something unknown (an unmodelled
// gate, or one of the many unlikely suspects not kept individually). A suspect's chance to be the
// cause is its prior weight (locality.ts) times its propensity, and propensities are re-estimated
// from the blame each cause took over all characters. The real blocker explains every character's
// block, so it takes the blame from proxies that only explain some of them, such as quests that
// players usually do right after the real blocker.

import type { Evidence } from "../../core/types";
import type { BlockedQuest } from "./blocked";
import type { AbsenceParams } from "./params";

export interface Share {
  /** Probability that this suspect caused the character's blocked moment. */
  share: number;
  /** The character had been offered the quest before (see BlockedMoment.transition). */
  transition: boolean;
  evidence: Evidence;
}

const UNKNOWN = "";

/** Suspect key -> character -> the character's most convincing moment for it. */
export function attribute(blocked: BlockedQuest, params: Pick<AbsenceParams, "unknownWeight" | "emIterations">): Map<string, Map<string, Share>> {
  const { moments } = blocked;
  // Every character counts once, however many moments they contributed.
  const momentsPerCharacter = new Map<string, number>();
  for (const moment of moments) momentsPerCharacter.set(moment.character, (momentsPerCharacter.get(moment.character) ?? 0) + 1);

  let propensity = new Map<string, number>([[UNKNOWN, 1]]);
  for (const moment of moments) for (const suspect of moment.suspects) propensity.set(suspect.key, 1);

  const unknownWeightOf = (residualWeight: number) => params.unknownWeight + residualWeight;
  const responsibilities = (moment: (typeof moments)[number], current: Map<string, number>): number[] => {
    const raw = moment.suspects.map((suspect) => suspect.weight * current.get(suspect.key)!);
    const total = raw.reduce((sum, value) => sum + value, unknownWeightOf(moment.residualWeight) * current.get(UNKNOWN)!);
    return raw.map((value) => value / total);
  };

  for (let iteration = 0; iteration < params.emIterations; iteration++) {
    const blame = new Map<string, number>();
    let totalBlame = 0;
    for (const moment of moments) {
      const weight = 1 / momentsPerCharacter.get(moment.character)!;
      const shares = responsibilities(moment, propensity);
      let explained = 0;
      moment.suspects.forEach((suspect, i) => {
        blame.set(suspect.key, (blame.get(suspect.key) ?? 0) + weight * shares[i]);
        explained += shares[i];
      });
      blame.set(UNKNOWN, (blame.get(UNKNOWN) ?? 0) + weight * (1 - explained));
      totalBlame += weight;
    }
    const next = new Map<string, number>();
    // A small floor keeps every suspect in play; the data decides, not the starting point.
    for (const key of propensity.keys()) next.set(key, Math.max((blame.get(key) ?? 0) / totalBlame, 1e-6));
    propensity = next;
  }

  const result = new Map<string, Map<string, Share>>();
  for (const moment of moments) {
    const shares = responsibilities(moment, propensity);
    moment.suspects.forEach((suspect, i) => {
      let byCharacter = result.get(suspect.key);
      if (!byCharacter) result.set(suspect.key, (byCharacter = new Map()));
      const best = byCharacter.get(moment.character);
      if (best && best.share >= shares[i]) return;
      const others = moment.suspects.length - 1;
      const text = `${moment.what}; ${suspect.detail}` + (others > 0 ? ` (${others} other suspects)` : "");
      byCharacter.set(moment.character, { share: shares[i], transition: moment.transition, evidence: { ref: moment.ref, t: moment.t, text } });
    });
  }
  return result;
}
