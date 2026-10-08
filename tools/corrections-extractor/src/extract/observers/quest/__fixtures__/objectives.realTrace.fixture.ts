// Trimmed excerpt of a real captured session (source trace file kept locally,
// not committed - see AGENTS.md "Forbidden Patterns: Don't commit trace files
// or dumps to repo"). This fixture preserves the exact real shapes/timings for
// a single quest objective end-to-end so regressions in objective-name parsing
// or evidence timing are caught by a realistic sample, not just hand-written
// data.
//
// Quest 92461 "Slay 8 Vuldren Juveniles in Thendal Grove.": the player targets
// a "Juvenile Vuldren" (Creature id 250873) well after the quest objective was
// first sampled at 0/8 - evidence and objective sample are NOT time-aligned,
// which is the realistic case this fixture is meant to guard against.

import type { SessionRecord } from "../../../../core/types";

export const objectivesRealTraceFixture: SessionRecord = {
  schemaVersion: 9,
  name: "real-trace-excerpt",
  startedAt: 0,
  startedAtPrecise: 0,
  stoppedAt: 400,
  stoppedAtPrecise: 400,
  duration: 400,
  durationPrecise: 400,
  events: [],
  functionsDelta: {},
  functions: {
    GetLocale: { player: [{ t: 0, tp: 0.005454300029669, v: "enUS" }] },
    "C_QuestLog.GetQuestObjectives": {
      "92461": [
        {
          t: 31.988000000012,
          tp: 31.720352599979,
          v: [
            {
              type: "monster",
              numRequired: 8,
              finished: false,
              text: "0/8 Juvenile Vuldren slain",
              objectiveType: 0,
              numFulfilled: 0,
            },
          ],
        },
        {
          t: 297.72100000002,
          tp: 297.2064575,
          v: [
            {
              type: "monster",
              numRequired: 8,
              finished: false,
              text: "1/8 Juvenile Vuldren slain",
              objectiveType: 0,
              numFulfilled: 1,
            },
          ],
        },
      ],
    },
    UnitGUID: {
      target: [
        // Unrelated NPC targeted earlier in the session (noise).
        { t: 288.20600000001, tp: 287.69539800001, v: "Creature-0-6782-2991-380-251368-000031C01F" },
        // The quest-objective kill: targeted well after the 0/8 objective sample.
        { t: 296.804, tp: 296.29263139999, v: "Creature-0-6782-2991-380-250873-0000326AFF" },
      ],
    },
    UnitName: {
      target: [
        { t: 288.20600000001, tp: 287.69539800001, v: { 1: "Elatrell Featherlight", n: 2 } },
        { t: 296.804, tp: 296.29263139999, v: { 1: "Juvenile Vuldren", n: 2 } },
      ],
    },
  },
};

export const JUVENILE_VULDREN_CREATURE_ID = 250873;
export const SLAY_VULDREN_QUEST_ID = 92461;
