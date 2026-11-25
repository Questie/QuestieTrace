# Schema changelog

What changed in the recorded data with each `SCHEMA_VERSION`, newest first. A session's `schemaVersion` says which rules it was recorded by. Use this to tell which sessions have a piece of data, whether an older one is worth less, and which form to prefer where a better one came in.

**When you change what or how something is recorded, add a line here in the same PR**, under the version it ships in: the current `SCHEMA_VERSION` while that is unreleased, else the next one. Say what changed, what it replaces, and what older sessions have instead, so a consumer can apply the better form to new sessions and fall back for old ones.

**A version alone does not always tell.** Several releases share one version, so a later release can have added something without a new number. Those lines say *from vX.Y.Z*. For them, look at whether the stream is in the session, not at its version. Sessions do not say which release recorded them.

- **Added**: a new stream, field or event.
- **Changed**: the same name holds something else, or more of it.
- **Removed**: it is no longer recorded. Old sessions still have it.
- **Consumers**: what that means for reading old and new sessions together.

Streams are listed with their exact names in [SCHEMA_SPEC.md](./SCHEMA_SPEC.md). The entries were checked against the streams in the code of each release tag and the release that first contains each commit.

Releases and their version: 9 = v1.0.0, v1.0.1 · 10 = v1.1.0, v1.1.1 · 11 = v1.2.0 · 12 = v1.3.0, v1.4.0 · 13 = v1.5.0, v1.6.0 · 14 = v1.6.1 · 15 = v1.7.0 · 16 = v1.8.0 to v1.9.1 · 17 = v1.9.2 · 18 = v1.10.0 to v1.12.0 · 19 = v1.13.0, v1.13.1 · 20 = v1.13.2 · 21 = v1.14.0 · 22 = not released.

## 22 (unreleased)

- **Added** `PlayerKnownSpells` through `C_SpellBook` and `C_SkillInfo.GetSkillLineInfoByID` on Forever, where neither spells nor skills were recorded before. Skills are keyed by skill ID, also those under a collapsed header.
- **Added** `C_Item.GetItemIDByGUID[itemGUID]` for item GUIDs in `UnitGUID`: the item that starts a quest. **Consumers**: Forever's `QUEST_DETAIL` reports `questStartItemID` 0, so this is the only source.
- **Added** `UnitGUIDIsSecret[token]`: whether the client hid which unit a token was. **Consumers**: below 22 a hidden unit looks like no unit, or like the unit before it.
- **Added** `IsQuestSequenced`. **Consumers**: there is nothing like it in older sessions.
- **Added** `TrainerServices[npcID][service]`, the trainer's kind and the list filters.
- **Added** `GetMerchantItemID`, `GetMerchantFilter`, and merchant samples when an item's data arrives late. **Consumers**: a third of the older slot series have no name or link.
- **Added** `UnitIsGhost["player"]`, `PLAYER_DEAD` and `PLAYER_UNGHOST`, the trivial quest range (`C_QuestLog.GetTrivialRange` on Forever, else `GetQuestGreenRange`), hardcore and season state, interaction windows (`PLAYER_INTERACTION_MANAGER_FRAME_SHOW`/`_HIDE`), and whether recipes used as quest requirements are known.
- **Added** the amount of reward currencies on Forever (`C_QuestOffer.GetQuestRewardCurrencyInfo`).
- **Changed** `IsBreadcrumbQuest` is recorded only while the player is on the quest. **Consumers**: below 22 it is false for dialog and offered quests, which is not the answer.
- **Changed** Functions that another addon brings back on a client without them are no longer recorded as the client's. **Consumers**: below 22, `GetFactionInfoByID`, `GetQuestLogTitle` and `GetQuestLogIndexByID` on Forever may hold another addon's answer.
- **Changed** Event arguments with names or party GUIDs are no longer stored (`QUEST_ACCEPT_CONFIRM`, `GROUP_JOINED`, `GROUP_LEFT`), and `UI_INFO_MESSAGE` texts are sanitized. **Consumers**: exports of older sessions scrub the first three events. The texts of `UI_INFO_MESSAGE` in them are **not** scrubbed and can hold a name.
- **Removed** `C_TradeSkillUI.GetAllProfessionTradeSkillLines` and `C_TradeSkillUI.GetTradeSkillLineInfoByID`, which no client has.

## 21 (v1.14.0)

- **Added** `UnitSubNameGUID`: the NPC a `UnitSubName` was shown for, at the same index. **Consumers**: below 21 a subname has no NPC, and belongs to whichever NPC was hovered last.
- **Added** `UPDATE_MOUSEOVER_UNIT`, for NPCs, and once when the mouse leaves one. The trackers listened to it before, but the addon never registered it. **Consumers**: below 21 the `mouseover` streams were only sampled when something else happened, so a hover can be missing or late.
- **Changed** Level lines in NPC tooltips are recognised by their text as well as their type, so on Forever they are not taken for a title. **Consumers**: below 21 a `UnitSubName` on Forever can be a level line.

## 20 (v1.13.2)

- Nothing in the recorded data changed from v1.13.1, which is version 19.

## 19 (v1.13.0, v1.13.1)

- **Added** `GameObjectGUID`, the GUID of the tooltip a `GameObjectName` came from (always `""` on Forever).
- **Changed** `GameObjectName` on Forever is the first tooltip line when the client gives no name line. **Consumers**: below 19 `GameObjectName` is empty on Forever. In a sample, none of 48 sessions of versions 16 to 18 had a name, and 87% of 420 from 19 on.
- **Changed** *from v1.13.1* names of crafters in tradeskill chat messages are redacted, and `UnitName` and `UnitGUID` are skipped when the client answers with a secret value.

## 18 (v1.10.0 to v1.12.0)

- **Added** *from v1.12.0* `UnitAffectingCombat` and `CheckInteractDistance["target"][4]`.
- **Removed** `QuestAcceptLevel`.

## 17 (v1.9.2)

- **Changed** Quest samples are spread out and stop once they settle, so a burst of quest events gives fewer samples, and completed quests are compared until the follow-up after a turn-in has settled, so late server data is no longer missed.

## 16 (v1.8.0 to v1.9.1)

- **Added** `UnitSubName` (NPC titles) and `GetMerchantItemLink`.
- **Changed** Player units on interaction tokens are recorded as no unit.
- **Changed** *from v1.9.0* the quest log's ID list is recorded at t=0. **Consumers**: before that the first list can be late.

## 15 (v1.7.0)

- **Added** quest rewards, item counts (`C_Item.GetItemCount`), item data arrival (`C_Item.IsItemDataCachedByID`), nameplates, display settings, the calendar time, explored map textures, completion texts and special items of quests, quest timer slots, reward currencies on Classic, experience, rested bonus and money, frame gaps (`frameUpdates`), full build info and `IsIndoors`.
- **Changed** Nameplate GUIDs are sampled in instances too, and a withheld or failed one is recorded as nil. **Consumers**: the other unit GUIDs are not recorded in instances (see 12).
- **Changed** Player pets and minions are no longer recorded as unit interactions, the player's guild name is redacted, and player names are redacted as whole words only.
- **Removed** The details of faction 1168, the player's guild. Its ID stays in `FactionOrder`.

## 14 (v1.6.1)

- **Changed** System messages in instances no longer raise an error; nothing in the recorded streams changed otherwise.

## 13 (v1.5.0, v1.6.0)

- **Added** *from v1.6.0* quest IDs of greetings and Forever's quest flags.

## 12 (v1.3.0, v1.4.0)

- **Added** `UnitLevel`, `UnitClassification` and `UnitReaction` of NPCs, and mouseover NPC interactions.
- **Added** *from v1.4.0* vendor items (`GetMerchantItemInfo`, `C_MerchantFrame.GetItemInfo`), quest POIs (`C_QuestLog.GetQuestsOnMap`), `GameObjectName`, and the player's level at quest acceptance (`QuestAcceptLevel`, removed in 18).
- **Changed** `GetLootSourceInfo` and `GetLootSlotLink` keep every successful sample instead of only changes, so loot can be joined by its exact time. **Consumers**: joining older sessions by time can silently lose loots.
- **Changed** *from v1.4.0* GUIDs are not recorded in instances.

## 11 (v1.2.0)

- **Added** `GetBuildInfo`. **Consumers**: below 11 nothing says which client a session is from.

## 10 (v1.1.0, v1.1.1)

- **Added** `QuestLogZone`, `C_DateAndTime.GetSecondsUntilDailyReset`, `GetQuestLogIndexByID`, `C_QuestLog.IsComplete`, `C_QuestLog.IsFailed`, `C_QuestLog.GetInfo`, `C_QuestLog.GetLogIndexForQuestID`, `C_QuestLog.GetQuestTagInfo` and the trade skill functions that no client has (removed in 22).
- **Changed** Streams with a compatibility layer are recorded as the raw functions each client has, such as `C_Reputation.GetFactionDataByID` next to `GetFactionInfoByID`, and `C_QuestLog.GetAllCompletedQuestIDs` next to `GetQuestsCompleted` (both delta streams).

## 9 (v1.0.0, v1.0.1)

- **Added** *from v1.0.1* the locale (`GetLocale`). Names and texts were already sanitized before they are stored.
- Anything earlier is not covered.
