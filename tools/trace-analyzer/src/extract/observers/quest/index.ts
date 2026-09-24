// Composes all quest field observers. One file per field, re-exported here for
// convenient bulk import by the extract pipeline.

export { observeName } from "./name";
export { observeQuestLevel } from "./questLevel";
export { observeRequiredLevel } from "./requiredLevel";
export { observeZoneOrSort } from "./zoneOrSort";
export { observeObjectives, mergeQuestObjectives } from "./objectives";
export { observeStartedBy, mergeStartedBy } from "./startedBy";
export { observeFinishedBy, mergeFinishedBy } from "./finishedBy";
