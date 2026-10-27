// Composes all npc field observers. One file per field, re-exported here for
// convenient bulk import by the extract pipeline.

export { observeName } from "./name";
export { observeLevel, mergeLevelRange } from "./level";
export { observeSpawns } from "./spawns";
export { observeZoneID } from "./zoneID";
export { observeFriendlyToFaction, mergeFriendlyToFaction } from "./friendlyToFaction";
export { observeRank, mergeRank } from "./rank";
export { mergeSpawns, mergeZoneID } from "./spawnsMerge";
