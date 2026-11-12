// Composes all quest field observers. One file per field, re-exported here for
// convenient bulk import by the extract pipeline.

export { observeName, observeLocalizedName } from "./name";
export { observeQuestLevel } from "./questLevel";
export { observeRequiredLevel } from "./requiredLevel";
export { observeZoneOrSort } from "./zoneOrSort";
export { observeObjectives } from "./objectives";
export { observeObjectiveCount } from "./objectiveCount";
export { observeObjectivesText, observeLocalizedObjectivesText } from "./objectivesText";
export { observeTriggerEnd, type QuestTriggerEndValue } from "./triggerEnd";
export { observeStartedBy } from "./startedBy";
export { observeFinishedBy } from "./finishedBy";
export { observeTurnInPoi } from "./turnInPoi";
export { observeReputationReward } from "./reputationReward";
