// Composes all quest field observers. One file per field, re-exported here for
// convenient bulk import by the extract pipeline.

export { observeName, observeLocalizedName } from "./name";
export { observeQuestLevel } from "./questLevel";
export { observeRequiredLevel } from "./requiredLevel";
export { observeZoneOrSort } from "./zoneOrSort";
export { observeObjectives, mergeQuestObjectives } from "./objectives";
export { observeObjectiveCount } from "./objectiveCount";
export { observeObjectivesText, observeLocalizedObjectivesText } from "./objectivesText";
export { observeTriggerEnd, mergeQuestTriggerEnds, type QuestTriggerEndValue } from "./triggerEnd";
export { observeStartedBy, mergeStartedBy } from "./startedBy";
export { observeFinishedBy, mergeFinishedBy } from "./finishedBy";
export { observeTurnInPoi, turnInPoiSpawns } from "./turnInPoi";
