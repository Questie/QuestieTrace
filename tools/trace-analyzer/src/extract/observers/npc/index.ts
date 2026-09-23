// Composes all npc field observers. One file per field, re-exported here for
// convenient bulk import by the extract pipeline.

export { observeName } from "./name";
export { observeMinLevel } from "./minLevel";
export { observeMaxLevel } from "./maxLevel";
export { observeQuestStarts, mergeQuestStarts } from "./questStarts";
