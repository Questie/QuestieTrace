// Composes all npc field observers. One file per field, re-exported here for
// convenient bulk import by the extract pipeline.

export { observeName } from "./name";
export { observeMinLevel } from "./minLevel";
export { observeMaxLevel } from "./maxLevel";
export { observeSpawns } from "./spawns";
export { observeZoneID } from "./zoneID";
export { mergeSpawns, mergeZoneID } from "./spawnsMerge";
export { observeQuestStarts, mergeQuestStarts } from "./questStarts";
export { observeQuestEnds, mergeQuestEnds } from "./questEnds";
