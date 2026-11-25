# Schema changelog

What changed in the recorded data with each `SCHEMA_VERSION`, newest first. A session's `schemaVersion` says which rules it was recorded by. Use this to tell which sessions have a piece of data, whether an older one is worth less, and which form to prefer where a better one came in.

**When you change what or how something is recorded, add a line here in the same PR**, under the version it ships in: the current `SCHEMA_VERSION` while that is unreleased, else the next one. Say what changed, what it replaces, and what older sessions have instead, so a consumer can apply the better form to new sessions and fall back for old ones.

- **Added**: a new stream, field or event. Sessions below the version do not have it.
- **Changed**: the same name holds something else, or more of it.
- **Removed**: it is no longer recorded. Old sessions still have it.
- **Consumers**: what that means for reading old and new sessions together.

Streams are listed with their exact names in [SCHEMA_SPEC.md](./SCHEMA_SPEC.md). Each entry was compiled from the commits of that version and from the streams in the code before and after.

Released versions: 9 in v1.0.0, 10 in v1.1.0, 11 in v1.2.0, 12 in v1.3.0, 13 in v1.5.0, 14 in v1.6.1, 15 in v1.7.0, 16 in v1.8.0, 17 in v1.9.2, 18 in v1.10.0, 19 in v1.13.0, 20 in v1.13.2, 21 in v1.14.0. 22 is not released.

## 22 (unreleased)

- **Added** `PlayerKnownSpells` through `C_SpellBook` and `C_SkillInfo.GetSkillLineInfoByID` on Forever, where neither spells nor skills were recorded before. Skills are keyed by skill ID, also for those under a collapsed header.
- **Added** `C_Item.GetItemIDByGUID[itemGUID]` for item GUIDs in `UnitGUID`: the item that starts a quest. **Consumers**: Forever's `QUEST_DETAIL` reports `questStartItemID` 0, so this is the only source.
- **Added** `UnitGUIDIsSecret[token]`: whether the client hid which unit a token was. **Consumers**: below 22 a hidden unit looks like no unit, or like the unit before it.
- **Added** `TrainerServices[npcID][service]`, the trainer's kind and the list filters.
- **Added** `GetMerchantItemID`, `GetMerchantFilter`, and merchant samples when an item's data arrives late. **Consumers**: a third of the older slot series have no name or link.
- **Added** `UnitIsGhost["player"]`, `PLAYER_DEAD` and `PLAYER_UNGHOST`, the trivial quest range (`C_QuestLog.GetTrivialRange` on Forever, else `GetQuestGreenRange`), hardcore and season state, interaction windows (`PLAYER_INTERACTION_MANAGER_FRAME_SHOW`/`_HIDE`), and whether recipes used as quest requirements are known.
- **Added** the amount of reward currencies on Forever (`C_QuestOffer.GetQuestRewardCurrencyInfo`).
- **Changed** `UPDATE_MOUSEOVER_UNIT` is recorded for NPCs only, plus once when the mouse leaves one. **Consumers**: below 22 the event also has players and other units.
- **Changed** `IsBreadcrumbQuest` and `IsQuestSequenced` are recorded only while the player is on the quest. **Consumers**: below 22 `IsBreadcrumbQuest` is false for dialog and offered quests, which is not the answer.
- **Changed** Functions that another addon brings back on a client without them are no longer recorded as the client's. **Consumers**: below 22, `GetFactionInfoByID`, `GetQuestLogTitle` and `GetQuestLogIndexByID` on Forever may hold another addon's answer.
- **Removed** `C_TradeSkillUI.GetAllProfessionTradeSkillLines` and `C_TradeSkillUI.GetTradeSkillLineInfoByID`, which no client has.
- **Changed** Event arguments with names or party GUIDs are no longer stored (`QUEST_ACCEPT_CONFIRM`, `GROUP_JOINED`, `GROUP_LEFT`, `UI_INFO_MESSAGE` texts). Exports of older sessions scrub them.

## 21

- **Added** `UnitSubNameGUID`: the NPC a `UnitSubName` was shown for, at the same index. **Consumers**: below 21 a subname has no NPC, and belongs to whichever NPC was hovered last.
- **Changed** Level lines in NPC tooltips are recognised by their text as well as their type, so on Forever they are not taken for a title. **Consumers**: below 21 a `UnitSubName` on Forever can be a level line.
- **Added** `UPDATE_MOUSEOVER_UNIT`, which the trackers listened to but the addon never registered. **Consumers**: below 21 the `mouseover` streams were only sampled when something else happened, so a hover can be missing or late.

## 20

- **Changed** Names of crafters in tradeskill chat messages are redacted.
- **Changed** `UnitName` and `UnitGUID` are skipped when the client answers with a secret value.

## 19

- **Added** `UnitAffectingCombat`, `CheckInteractDistance["target"][4]` and `GameObjectGUID`, the GUID of the tooltip a `GameObjectName` came from (always `""` on Forever).
- **Changed** `GameObjectName` on Forever is the first tooltip line when the client gives no name line. **Consumers**: below 19 `GameObjectName` is empty on Forever. In 48 sessions of schema 16 to 18 none had a name; from 19, 87% of 420 sessions do.

## 18

- **Removed** `QuestAcceptLevel`.
- **Changed** Completed quests are sampled until the follow-up after a turn-in has settled, so late server data is no longer missed.

## 17

- **Added** `UnitSubName` (NPC titles) and `GetMerchantItemLink`.
- **Changed** The quest log's ID list is recorded at t=0, and quest samples are spread out and settle, so a burst of quest events gives fewer samples.

## 16

- **Added** item data arrival (`C_Item.IsItemDataCachedByID`), experience, rested bonus and money, frame gaps (`frameUpdates`), quest timer slots, build info, reward currencies on Classic, and `IsIndoors`.
- **Changed** Player units on interaction tokens are recorded as no unit.
- **Changed** Player names are redacted as whole words only.

## 15

- **Added** quest rewards, item counts (`C_Item.GetItemCount`), nameplates, display settings, the calendar time, explored map textures, completion texts and special items of quests.
- **Changed** Player pets and minions are no longer recorded as unit interactions, and the player's guild name is redacted.

## 14

- **Added** quest IDs of greetings, and Forever's quest flags.

## 13

- **Added** vendor items (`GetMerchantItemInfo`, `C_MerchantFrame.GetItemInfo`), quest POIs (`C_QuestLog.GetQuestsOnMap`), `GameObjectName` and the player's level at quest acceptance (`QuestAcceptLevel`, removed in 18).
- **Changed** GUIDs are not recorded in instances.

## 12

- **Added** `UnitLevel`, `UnitClassification` and `UnitReaction` of NPCs, and mouseover NPC interactions.

## 11

- **Added** `GetBuildInfo`. **Consumers**: below 11 nothing says which client a session is from.

## 10

- **Added** names and texts run through the sanitizer before they are stored, and the locale.
- **Changed** Streams with a compatibility layer are recorded as the raw functions each client has: `C_Reputation.GetFactionDataByID` and `GetFactionInfoByID`, `C_QuestLog.GetAllCompletedQuestIDs` and `GetQuestsCompleted` (both delta streams), and the skill and quest log functions.
