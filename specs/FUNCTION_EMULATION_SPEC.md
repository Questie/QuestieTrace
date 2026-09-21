# Function Emulation Spec (v9)

How to reconstruct WoW API function outputs at a target time `t` from QuestieTrace saved data.

The observed-only raw API guarantee applies to sessions with
`recordingContractVersion = 1`. Unmarked sessions, including older schema v9
captures, have legacy/unknown semantics and may contain synthetic resets. Treat
unsupported contract versions as unknown; do not infer a marker from values.
Lookup mechanics remain the same, but a last observation is not proof of the
API's value at a later time, especially after a failed probe.

## 1) Generic lookup

Stored function streams use the same lookup algorithm: find the stream for an API/function key, follow any parameter maps in native argument order, find the latest entry at or before the target time, and unpack packed tuple values when returning them.

```lua
-- Multi-argument APIs use nested parameterized tables in native argument order.
function getStream(session, name, ...)
  local fn = session.functions[name]
  if not fn then return nil end
  for i = 1, select("#", ...) do
    local param = select(i, ...)
    if param ~= nil then
      fn = fn[param]
      if not fn then return nil end
    end
  end
  return fn
end

function valueAt(stream, target_t)
  local result = nil
  for i = 1, #stream do
    if stream[i].t > target_t then break end
    result = stream[i].v
  end
  return result
end

function emulate(v)
  if type(v) == "table" and v.n then
    return unpack(v, 1, v.n)
  end
  return v
end
```

Direct stream lookup does not need per-function logic. WoW-signature emulation for APIs whose native parameters differ from the stored key, such as `GetQuestLogTitle(index)` or `GetFactionInfo(index)`, requires derived lookup logic.

## 2) Parameterless vs parameterized detection

In Lua-shaped data, a table whose first child has `t` is a leaf stream array; otherwise it is a parameter map. Nested streams apply this rule recursively.

```lua
local fn = session.functions[name]
local firstKey = next(fn)
if type(fn[firstKey]) == "table" and fn[firstKey].t then
  -- flat stream array
else
  -- argument-keyed map; index/recurse by params
end
```

The TypeScript analyzer normalizes Lua tables first. Leaf streams become arrays and parameter maps remain objects keyed by parameter. Numeric Lua keys are normalized to strings by the loader, so analyzer lookup stringifies parameters while walking nested maps.

## 3) Function-to-stream mapping

Every stored function maps directly to `session.functions[key]`, `session.functions[key][param]`, or `session.functions[key][arg1][arg2]...`.

| WoW API call | Stream lookup |
|---|---|
| `GetZoneText()` | `functions["GetZoneText"]` |
| `GetSubZoneText()` | `functions["GetSubZoneText"]` |
| `GetRealZoneText()` | `functions["GetRealZoneText"]` |
| `IsInInstance()` | `functions["IsInInstance"]` |
| `GetInstanceInfo()` | `functions["GetInstanceInfo"]` |
| `GetNumLootItems()` | `functions["GetNumLootItems"]` |
| `IsInGroup()` | `functions["IsInGroup"]` |
| `GetNumGroupMembers()` | `functions["GetNumGroupMembers"]` |
| `GetNumSkillLines()` | `functions["GetNumSkillLines"]` (only present when this legacy global exists; no synthesized fallback) |
| `GetSkillLineInfo(index)` (legacy) | `functions["GetSkillLineInfo"][index]` (only present when `GetNumSkillLines` exists) |
| `C_TradeSkillUI.GetAllProfessionTradeSkillLines()` | `functions["C_TradeSkillUI.GetAllProfessionTradeSkillLines"]` |
| `C_TradeSkillUI.GetTradeSkillLineInfoByID(skillLineID)` | `functions["C_TradeSkillUI.GetTradeSkillLineInfoByID"][skillLineID]` |
| `GetProfessions()` | `functions["GetProfessions"]` |
| `GetProfessionInfo(index)` | `functions["GetProfessionInfo"][index]` |
| `C_QuestLog.GetMaxNumQuestsCanAccept()` | `functions["C_QuestLog.GetMaxNumQuestsCanAccept"]` |
| `GetServerTime()` | `functions["GetServerTime"]` snapshot; derive continuous time if needed |
| `GetQuestResetTime()` (legacy) | `functions["GetQuestResetTime"]` snapshot; only present when this global exists |
| `C_DateAndTime.GetSecondsUntilDailyReset()` | `functions["C_DateAndTime.GetSecondsUntilDailyReset"]` snapshot; only present when this API exists |
| `C_GossipInfo.GetAvailableQuests()` | `functions["C_GossipInfo.GetAvailableQuests"]` |
| `C_GossipInfo.GetActiveQuests()` | `functions["C_GossipInfo.GetActiveQuests"]` |
| `C_GossipInfo.GetNumAvailableQuests()` | `functions["C_GossipInfo.GetNumAvailableQuests"]` |
| `C_GossipInfo.GetNumActiveQuests()` | `functions["C_GossipInfo.GetNumActiveQuests"]` |
| `C_GossipInfo.GetText()` | `functions["C_GossipInfo.GetText"]` |
| `C_GossipInfo.GetOptions()` | `functions["C_GossipInfo.GetOptions"]` |
| `GetNumGossipAvailableQuests()` | `functions["GetNumGossipAvailableQuests"]` |
| `GetNumGossipActiveQuests()` | `functions["GetNumGossipActiveQuests"]` |
| `GetGossipAvailableQuests()` | `functions["GetGossipAvailableQuests"]` |
| `GetGossipActiveQuests()` | `functions["GetGossipActiveQuests"]` |
| `GetGreetingText()` | `functions["GetGreetingText"]` |
| `GetNumActiveQuests()` | `functions["GetNumActiveQuests"]` |
| `GetActiveTitle(index)` | `functions["GetActiveTitle"][index]` |
| `GetNumAvailableQuests()` | `functions["GetNumAvailableQuests"]` |
| `GetAvailableTitle(index)` | `functions["GetAvailableTitle"][index]` |
| `GetQuestID()` | `functions["GetQuestID"]` |
| `GetTitleText()` | `functions["GetTitleText"]` |
| `GetQuestText()` | `functions["GetQuestText"]` |
| `GetObjectiveText()` | `functions["GetObjectiveText"]` |
| `GetProgressText()` | `functions["GetProgressText"]` |
| `GetRewardText()` | `functions["GetRewardText"]` |
| `GetRewardXP()` | `functions["GetRewardXP"]` |
| `IsQuestCompletable()` | `functions["IsQuestCompletable"]` |
| `GetNumQuestChoices()` | `functions["GetNumQuestChoices"]` |
| `UnitLevel("player")` | `functions["UnitLevel"]["player"]` |
| `UnitRace("player")` | `functions["UnitRace"]["player"]` |
| `UnitClass("player")` | `functions["UnitClass"]["player"]` |
| `UnitClassBase("player")` | `functions["UnitClassBase"]["player"]` |
| `UnitSex("player")` | `functions["UnitSex"]["player"]` |
| `UnitFactionGroup("player")` | `functions["UnitFactionGroup"]["player"]` |
| `C_Map.GetBestMapForUnit("player")` | `functions["C_Map.GetBestMapForUnit"]["player"]` |
| `C_Map.GetPlayerMapPosition(map, "player")` | `functions["C_Map.GetPlayerMapPosition"]["player"]` |
| `UnitGUID(token)` | `functions["UnitGUID"][token]` |
| `UnitName(token)` | `functions["UnitName"][token]` |
| `IsQuestComplete(questId)` | `functions["IsQuestComplete"][questId]` |
| `HaveQuestData(questId)` | `functions["HaveQuestData"][questId]` |
| `C_QuestLog.IsOnQuest(questId)` | `functions["C_QuestLog.IsOnQuest"][questId]` |
| `C_QuestLog.IsQuestFlaggedCompleted(questId)` | `functions["C_QuestLog.IsQuestFlaggedCompleted"][questId]` |
| `C_QuestLog.GetQuestObjectives(questId)` | `functions["C_QuestLog.GetQuestObjectives"][questId]` |
| `C_QuestLog.GetInfo(questLogIndex)` for active quest `questId` | `functions["C_QuestLog.GetInfo"][questId]` |
| `C_QuestLog.GetQuestTagInfo(questId)` | `functions["C_QuestLog.GetQuestTagInfo"][questId]` |
| `C_QuestLog.IsComplete(questId)` | `functions["C_QuestLog.IsComplete"][questId]` |
| `C_QuestLog.IsFailed(questId)` | `functions["C_QuestLog.IsFailed"][questId]` |
| `C_QuestLog.GetLogIndexForQuestID(questId)` | `functions["C_QuestLog.GetLogIndexForQuestID"][questId]` |
| `GetQuestLogTitle(questLogIndex)` (legacy) for active quest `questId` | `functions["GetQuestLogTitle"][questId]` (raw 17-value tuple, only present when this global exists) |
| `GetQuestLogIndexByID(questId)` (legacy) | `functions["GetQuestLogIndexByID"][questId]` |
| `GetQuestLogQuestText(questLogIndex)` for active quest `questId` | `functions["GetQuestLogQuestText"][questId]` |
| Timed quest value for `questId` | `functions["GetQuestTimers"][questId]` |
| Quest log time-left value for `questId` | `functions["GetQuestLogTimeLeft"][questId]` |
| `GetNumQuestLogRewards(questId)` | `functions["GetNumQuestLogRewards"][questId]` |
| `GetQuestLogRewardMoney(questId)` | `functions["GetQuestLogRewardMoney"][questId]` |
| `GetQuestLogRewardInfo(rewardIndex, questId)` | `functions["GetQuestLogRewardInfo"][rewardIndex][questId]` |
| `GetQuestTagInfo(questId)` | `functions["GetQuestTagInfo"][questId]` |
| `GetLootSlotInfo(slot)` | `functions["GetLootSlotInfo"][slot]` |
| `GetLootSourceInfo(slot)` | `functions["GetLootSourceInfo"][slot]` |
| `GetLootSlotLink(slot)` | `functions["GetLootSlotLink"][slot]` |
| `GetLootSlotType(slot)` | `functions["GetLootSlotType"][slot]` |
| `GetFactionInfoByID(factionID)` (legacy) | `functions["GetFactionInfoByID"][factionID]` (raw tuple, only present when `C_Reputation.GetFactionDataByID` is unavailable) |
| `C_Reputation.GetFactionDataByID(factionID)` | `functions["C_Reputation.GetFactionDataByID"][factionID]` (raw table) |
| `GetProfessionInfo(index)` | `functions["GetProfessionInfo"][index]` |
| `GetSpellBookItemName(slot)` | `functions["GetSpellBookItemName"][slot]` |
| `GetSpellBookItemInfo(slot)` | `functions["GetSpellBookItemInfo"][slot]` |
| `IsPassiveSpell(slot)` | `functions["IsPassiveSpell"][slot]` |

Notes:

- Raw questID API streams such as `C_QuestLog.IsQuestFlaggedCompleted`, `C_QuestLog.IsOnQuest`, `HaveQuestData`, objectives, reward count/money, `GetQuestTagInfo`, `C_QuestLog.GetInfo`, `C_QuestLog.GetQuestTagInfo`, `C_QuestLog.IsComplete`, and `C_QuestLog.IsFailed` store observed API returns. When a quest leaves `QuestLog`, the tracker runs post-invalidation probes and records only values returned by successful calls. `C_QuestLog.IsQuestFlaggedCompleted` is related to `GetQuestsCompleted`, but the raw stream is not derived from it.
- Quest-log title/tag/completion data is intentionally split across several independent raw streams (`C_QuestLog.GetInfo`, `C_QuestLog.GetQuestTagInfo`, `C_QuestLog.IsComplete`, `C_QuestLog.IsFailed`, legacy `GetQuestLogTitle`) rather than one normalized composite value. Consumers that want a single "quest log title" view must combine whichever of these streams are present themselves; a client that lacks one of the modern `C_QuestLog.*` APIs simply has no entries for that stream, it is never backfilled from another API.
- `GetQuestLogIndexByID`/`C_QuestLog.GetLogIndexForQuestID` and `GetQuestLogQuestText` are sampled with the native quest-log index but stored by `questId`. Native index emulation should map index → questId via the `QuestLog` synthetic stream first. After a quest leaves the active log, there is no valid quest-log index to probe for these streams.
- `GetQuestTimers[questId]` and `GetQuestLogTimeLeft[questId]` are derived compatibility streams from `GetQuestTimers()` and `GetQuestIndexForTimer(timerIndex)`; they do not mutate quest-log selection. Their nil entries indicate derived timer mapping invalidation, not raw native API returns.
- `GetQuestLogRewardInfo` uses nested maps in native argument order `[rewardIndex][questId]`. Previously observed reward indices are probed again after counts shrink or quests leave the log; stored values are successful API returns, not invented inactive values.
- Legacy gossip globals are raw packed varargs; `GetGossipAvailableQuests` is repeated 7-tuples and `GetGossipActiveQuests` is repeated 6-tuples. Quest dialog close events cancel pending delayed open/update reads and perform one observed API sample; any empty string, nil, zero, or table value in these raw streams is an API return, not a synthetic reset.
- Loot, skill/profession, spellbook, and unit-token streams follow the same raw-observation rule. Close events or shrinking index ranges cause event-synchronous probes of the represented APIs; failed calls are skipped rather than represented by invented values.
- `GetFactionInfoByID`/`C_Reputation.GetFactionDataByID` are two independent raw streams for the same conceptual data, not one normalized value: whichever real API the client exposes is probed and recorded under that API's own name.
- `GetNumSkillLines`/`GetSkillLineInfo` have no synthesized fallback: on clients lacking these legacy globals, those two streams simply have no entries. `GetProfessions`/`GetProfessionInfo` and `C_TradeSkillUI.GetAllProfessionTradeSkillLines`/`C_TradeSkillUI.GetTradeSkillLineInfoByID` are tracked independently and unconditionally whenever they exist (not only as a fallback for the legacy skill-line APIs).

### Stored custom streams

| Stream | Meaning |
|---|---|
| `functions["QuestLog"]` | Array of active quest IDs in quest-log order |
| `functions["FactionOrder"]` | Ordered array of known faction IDs |
| `functions["SpellBook"]` | Ordered unique spell IDs discovered from spellbook slots |

### Derived WoW APIs

| WoW API call | Derivation |
|---|---|
| `GetFactionInfo(index)` | `factionID = valueAt(functions["FactionOrder"], t)[index]` → `functions["GetFactionInfoByID"][factionID]` or `functions["C_Reputation.GetFactionDataByID"][factionID]`, whichever is present |
| `GetNumFactions()` | `#valueAt(functions["FactionOrder"], t)` |
| `GetQuestLogTitle(questLogIndex)` | `questId = valueAt(functions["QuestLog"], t)[questLogIndex]` → combine whichever of `functions["C_QuestLog.GetInfo"][questId]`, `functions["C_QuestLog.GetQuestTagInfo"][questId]`, `functions["C_QuestLog.IsComplete"][questId]`, `functions["C_QuestLog.IsFailed"][questId]`, or legacy `functions["GetQuestLogTitle"][questId]` are present |
| `GetQuestLogQuestText(questLogIndex)` | same index → questId map, then `functions["GetQuestLogQuestText"][questId]` |

## 4) Delta stream replay

`GetQuestsCompleted` and `PlayerKnownSpells` live in `functionsDelta`:

```lua
function getDeltaSet(session, key, target_t)
  local data = session.functionsDelta[key]
  local set = {}
  for _, id in ipairs(data.initial) do set[id] = true end
  for _, delta in ipairs(data.delta) do
    if delta.t > target_t then break end
    if delta.add then for _, id in ipairs(delta.add) do set[id] = true end end
    if delta.remove then for _, id in ipairs(delta.remove) do set[id] = nil end end
  end
  return set
end
```

## 5) Event replay

Iterate `session.events` in order and fire events whose `t` lies in the requested range. Position movement transitions (`PLAYER_STARTED_MOVING`, `PLAYER_STOPPED_MOVING`) are private sampling triggers and are not recorded in the main event stream.

## 6) Known limits

- Only captured/listed functions can be emulated.
- Missing `v` means a stored nil value.
- `C_Map.GetPlayerMapPosition["player"]` is a derived compatibility stream, not a raw API stream. It uses the current map, rounds XY to 4 decimal places, and stores nil when the map or position is unavailable. A missing map produces nil without calling the position API; that nil is not a native API observation.
- Skill line IDs are not normalized during capture.
- Reset-time streams are low-frequency snapshots; replay consumers derive continuously changing server time or countdown behavior.
