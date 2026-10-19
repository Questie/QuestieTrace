# Tracker Spec (v9)

How trackers are structured, registered, and dispatched. This is the core
architecture pattern for capturing WoW API function data.

---

## 1) TrackerDef interface

Every tracker file calls `Core.RegisterTracker(def)` once at file scope.
The definition table has four optional fields:

```lua
Core.RegisterTracker({
  events = { "EVENT_NAME", ... },                -- optional
  Init = function(capture) ... end,              -- optional
  OnEvent = function(capture, event, ...) end,   -- optional
  OnCaptureStopped = function(capture) end,      -- optional
})
```

| Field | Called when | Purpose |
|---|---|---|
| `events` | File load time | List of WoW events this tracker cares about |
| `Init` | `Core.StartCapture()` | Create initial function streams at t=0 |
| `OnEvent` | Event fires during capture | Sample functions, append if changed |
| `OnCaptureStopped` | `Core.StopCapture()` | Final sampling or cleanup |

If `events` is nil or absent, the tracker receives no event dispatches
(e.g. PlayerIdentity, which only samples at t=0).

---

## 2) Registration and event routing

`Core.RegisterTracker` (in `globals.lua`) does two things:

1. Appends the tracker to `Core._trackers` (ordered list).
2. For each event in `events`, appends `OnEvent` to
   `Core._trackerCallbacks[event]` (an `event -> callback[]` lookup).

At runtime, `ProcessTrackedEvent` in `QuestieTrace.lua`:

1. Records the raw event to `capture.session.events`.
2. Looks up `Core._trackerCallbacks[event]`.
3. Calls each registered callback with `(capture, event, ...)`.

Only trackers that registered for a given event run when it fires.

---

## 3) CaptureState object

The `capture` object passed to all tracker callbacks:

```lua
capture = {
  active          = true,       -- false after StopCapture
  token           = 1,          -- monotonically increasing, for timer invalidation
  startedAt       = 100000.0,   -- GetTime() at capture start (absolute)
  startedAtPrecise = 4821.312,  -- GetTimePreciseSec() at capture start (absolute)
  session         = {           -- the SessionRecord being built
    schemaVersion  = 9,
    name           = "session-name" or nil,
    startedAt      = 100000.0,
    startedAtPrecise = 4821.312,
    events         = {},
    functions      = {},
    functionsDelta = {},
  },
}
```

### Three-state machine

| `capture.active` | `capture.session` | State |
|---|---|---|
| `false` | `nil` | Idle — no session |
| `true` | table | Running — capture active |
| `false` | table | Stopped, unsaved |

### Token-based timer invalidation

`capture.token` increments on every `StartCapture`. Trackers that schedule
delayed callbacks (via `C_Timer.After`) capture the token at schedule time
and check it in the callback:

```lua
local token = capture.token
C_After(delay, function()
  if not capture.active or capture.token ~= token then return end
  -- safe to sample
end)
```

This ensures stale timers from a previous capture session are discarded.

---

## 4) Zero-copy architecture

Trackers write directly into `capture.session.functions` and
`capture.session.functionsDelta`. During `Init`, each tracker creates its
stream tables inside the session object and holds local references:

```lua
-- In Init:
capture.session.functions["UnitLevel"] = {
  ["player"] = { { t = 0, tp = 0, v = 12 } },
}
stream = capture.session.functions["UnitLevel"]["player"]

-- In OnEvent:
stream[#stream + 1] = { t = t, tp = tp, v = 13 }
```

While recording, the session object built by trackers is the SessionRecord
linked into SavedVariables as `currentSession`. On save it is encoded into an
`EncodedSessionRecord` (see SCHEMA_SPEC).

---

## 5) Tracker lifecycle

### On StartCapture

1. `capture.token` increments.
2. `capture.startedAt` and `capture.startedAtPrecise` are set.
3. `capture.session` is created (empty `events`, `functions`, `functionsDelta`).
4. `capture.active = true`.
5. `Init` is called on every registered tracker (in registration order).
6. No synthetic events are emitted.

### On each game event

1. Raw event is appended to `capture.session.events` with `{t, tp, e, a}`.
2. Registered tracker callbacks run.
3. Each tracker samples its functions and appends only if value changed.
4. All entries produced in the same call stack share the same `t` value
   (from `GetTime() - capture.startedAt`).

### On StopCapture

1. Stop timestamps are recorded on the session.
2. `capture.active = false`.
3. `OnCaptureStopped` is called on every registered tracker.
4. Timer-based trackers stop naturally (their callbacks check `capture.active`).

### On SaveCapture

1. Auto-stops if still running.
2. Session is encoded (see `EncodedSessionRecord` in SCHEMA_SPEC) and appended to `QuestieTraceCharacter.sessions`.
3. `capture.session` is set to `nil`.

---

## 6) Trigger patterns

Five distinct patterns exist across trackers:

### Pattern 1: Event-driven

Sample when a specific game event fires.

**Example:** UnitLevel — samples `UnitLevel("player")` on `PLAYER_LEVEL_UP`.

### Pattern 2: Timer-driven

Sample on a repeating interval regardless of events.

**Example:** Position — samples every 0.2 seconds via `C_Timer.After`.
The timer is started in `Init` and self-schedules until `capture.active`
becomes false.

### Pattern 3: Event + delayed re-samples

Sample immediately on event, then re-sample at staggered delays to catch
server-lag states where the API initially returns incomplete data.

**Schedule:** `{ 0, 0.10, 0.35, 0.55, 0.75, 1.00 }` seconds.

The 0-delay entry samples in the same call stack as the event. Subsequent
entries use `C_Timer.After` with token-based invalidation.

**Example:** QuestLog — quest objective text may show `" : 0/8"` before
the server sends the real item name. The delayed re-samples catch the
correct text when it arrives.

### Pattern 4: Event-driven with index iteration

When an event fires, the tracker iterates a dynamic set of keys to sample.

**Example:** QuestLog — iterates all quest IDs in the log and samples
each per-quest function for every ID. Reputation — iterates all known
factionIDs.

### Pattern 5: Event-driven with window lifecycle

Sample raw APIs on open/update events and again on close events. Close samples
record observed API returns only; they do not invent `nil`, `0`, `false`, empty
arrays, or empty packed tuples for raw streams.

**Example:** Loot — samples all slot functions on `LOOT_READY`, then probes
`GetNumLootItems` and known slot APIs on `LOOT_CLOSED`, appending only
successful observed returns.

---

## 7) Per-tracker design

| Tracker | File | Functions | Trigger | Events |
|---|---|---|---|---|
| PlayerIdentity | `Trackers/PlayerIdentity.lua` | `UnitRace["player"]`, `UnitClass["player"]`, `UnitClassBase["player"]`, `UnitSex["player"]`, `UnitFactionGroup["player"]` | Init only (t=0) | None |
| UnitLevel | `Trackers/UnitLevel.lua` | `UnitLevel["player"]`, `GetQuestGreenRange` | Event-driven | `PLAYER_LEVEL_UP`, `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| Position | `Trackers/Position.lua` | `GetZoneText`, `GetSubZoneText`, `GetRealZoneText`, `C_Map.GetBestMapForUnit["player"]`, `C_Map.GetPlayerMapPosition["player"]`, `IsInInstance`, `GetInstanceInfo`, `IsIndoors`, `C_MapExplorationInfo.GetExploredMapTextures[mapID]` (on map change and `MAP_EXPLORATION_UPDATED` only). `GetPlayerFacing` is not recorded; replay derives the heading from consecutive positions | Timer (0.2s) + event-driven + private movement sampling | zone/map events + `PLAYER_ENTERING_WORLD`, `PLAYER_ALIVE`, `SPELLS_CHANGED`; movement events are private sampling triggers and are not recorded globally |
| Loot | `Trackers/Loot.lua` | `GetNumLootItems`, `GetLootSlotInfo[slot]`, `GetLootSourceInfo[slot]`, `GetLootSlotLink[slot]`, `GetLootSlotType[slot]` | Event + window lifecycle | `LOOT_READY`, `LOOT_CLOSED` |
| Reputation | `Trackers/Reputation.lua` | `FactionOrder`, `GetFactionInfoByID[factionID]` and `C_Reputation.GetFactionDataByID[factionID]` (independent, each whenever its own API exists) for every `FactionOrder` faction except the guild faction | Event + index iteration | `CHAT_MSG_COMBAT_FACTION_CHANGE`, `UPDATE_FACTION`, `QUEST_TURNED_IN`, `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| QuestLog | `Trackers/QuestLog.lua` | `QuestLog`, `C_QuestLog.GetMaxNumQuestsCanAccept`, `IsQuestComplete[qid]`, `HaveQuestData[qid]`, `C_QuestLog.IsOnQuest[qid]`, `C_QuestLog.IsQuestFlaggedCompleted[qid]`, `C_QuestLog.GetQuestObjectives[qid]`, `C_QuestLog.GetInfo[qid]`, `C_QuestLog.GetQuestTagInfo[qid]`, `C_QuestLog.IsComplete[qid]`, `C_QuestLog.IsFailed[qid]`, `C_QuestLog.GetLogIndexForQuestID[qid]`, `GetQuestLogTitle[qid]`, `GetQuestLogIndexByID[qid]`, `GetQuestLogQuestText[qid]`, `GetQuestLogCompletionText[qid]`, `GetQuestLogSpecialItemInfo[qid]`, `C_QuestLog.GetRequiredMoney[qid]`, `QuestLogZone[qid]`, timer streams (`C_QuestLog.GetQuestTimers`, raw `GetQuestIndexForTimer[slot]` and the legacy derived ones), reward streams, `GetQuestTagInfo[qid]` | Event + delayed re-samples + index iteration | 14 quest events + `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| CompletedQuests | `Trackers/CompletedQuests.lua` | `C_QuestLog.GetAllCompletedQuestIDs`/`GetQuestsCompleted` (independent functionsDelta streams) | Event + delayed re-samples | Same 14 quest events + `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| UnitInteraction | `Trackers/UnitInteraction.lua` | `UnitGUID`/`UnitName` for `target`, `npc`, `questnpc`, `mouseover`; `UnitGUID` only for the `nameplateN` token of each added nameplate; `C_GossipInfo.GetAvailableQuests`; `C_GossipInfo.GetActiveQuests` | Event-driven fixed unit-token fanout; nameplates per event token | target, quest dialog, selected quest state, loot-open, NPC interaction, login-time events, `UPDATE_MOUSEOVER_UNIT`, `NAME_PLATE_UNIT_ADDED` |
| UnitState | `Trackers/UnitState.lua` | `UnitLevel`/`UnitClassification`/`UnitReaction` for `target`, `mouseover` | Event-driven fixed unit-token fanout (NPC-only) | `PLAYER_TARGET_CHANGED`, `UPDATE_MOUSEOVER_UNIT` |
| GroupState | `Trackers/GroupState.lua` | `IsInGroup`, `GetNumGroupMembers` | Event-driven | `GROUP_JOINED`, `GROUP_LEFT`, `GROUP_ROSTER_UPDATE`, `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| SkillLines | `Trackers/SkillLines.lua` | `GetNumSkillLines[?]`, `GetSkillLineInfo[index]` (legacy, when present), `GetProfessions`, `GetProfessionInfo[index]`, `C_TradeSkillUI.GetAllProfessionTradeSkillLines`, `C_TradeSkillUI.GetTradeSkillLineInfoByID[skillLineID]` | Event + index iteration | `SKILL_LINES_CHANGED`, `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| SpellBook | `Trackers/SpellBook.lua` | `SpellBook`, `GetSpellBookItemName[slot]`, `GetSpellBookItemInfo[slot]`, `IsPassiveSpell[slot]`, `PlayerKnownSpells` (functionsDelta) | Event + slot iteration | `SPELLS_CHANGED`, `PLAYER_ENTERING_WORLD` |
| QuestDialog | `Trackers/QuestDialog.lua` | Gossip, greeting, and current quest-dialog APIs, including reward/required counts, money, title, skill points, group size, auto-accept and background material; `GetQuestItemInfo[type][index]`, `GetQuestItemLink[type][index]`, Mists reward currencies (`GetQuestCurrencyInfo`/`GetQuestCurrencyID["reward"][index]`); `GetAvailableQuestInfo[index]`, `GetActiveQuestID[index]`; quest flags `IsBreadcrumbQuest[qid]`, `C_QuestLine.GetQuestLineInfo[qid]`, `C_QuestInfoSystem.GetQuestClassification[qid]` (Forever only) | Event + delayed re-samples + observed close sample | `QUEST_DETAIL`, `QUEST_PROGRESS`, `QUEST_COMPLETE`, `QUEST_FINISHED`, `QUEST_GREETING`, `QUEST_ACCEPT_CONFIRM`, `GOSSIP_SHOW`, `GOSSIP_CLOSED` |
| ResetTime | `Trackers/ResetTime.lua` | `GetServerTime`, `GetQuestResetTime` (legacy, when present), `C_DateAndTime.GetSecondsUntilDailyReset` and `C_DateAndTime.GetCurrentCalendarTime` (each when present) | Init + low-frequency event snapshots | `PLAYER_LOGIN`, `PLAYER_ENTERING_WORLD`, `PLAYER_LOGOUT` |
| ItemCount | `Trackers/ItemCount.lua` | `C_Item.GetItemCount[itemID]` | Init snapshot of carried items + event-driven + throttled private rescan | `ITEM_COUNT_CHANGED`; `BAG_UPDATE_DELAYED` is a private trigger and is not recorded |
| Display | `Trackers/Display.lua` | `GetPhysicalScreenSize`, `UIParent:GetScale`, `GetScreenWidth`, `GetScreenHeight`, `GetCVar[lowercase name]` (allowlisted CVars) | Init + event-driven + private CVar trigger | `PLAYER_ENTERING_WORLD`, `UI_SCALE_CHANGED`, `DISPLAY_SIZE_CHANGED`; `CVAR_UPDATE` is a private trigger and is not recorded |
| PlayerProgress | `Trackers/PlayerProgress.lua` | `UnitXP["player"]`, `UnitXPMax["player"]`, `GetXPExhaustion`, `GetMoney` | Init + event-driven | `PLAYER_ENTERING_WORLD`, `PLAYER_XP_UPDATE`, `PLAYER_LEVEL_UP`, `UPDATE_EXHAUSTION`, `PLAYER_MONEY` |
| ItemInfo | `Trackers/ItemInfo.lua` | `C_Item.IsItemDataCachedByID[itemID]` for items an addon asked about while uncached | `hooksecurefunc` post-hook on item data requests + private triggers | None; `GET_ITEM_INFO_RECEIVED` and `ITEM_DATA_LOAD_RESULT` are private triggers and are not recorded |
| FrameHeartbeat | `Trackers/FrameHeartbeat.lua` | None; writes `session.frameUpdates` | `OnUpdate` while capturing | None |
| GameObject | `Trackers/GameObject.lua` | `GameObjectName` (synthetic) | TooltipDataProcessor callback (Modern/Forever only) | None (uses `TooltipDataProcessor.AddTooltipPostCall`) |

---

## 8) Reputation tracker behavior

The Reputation tracker has unique complexity that warrants explicit
documentation.

### Collection vs sampling

Two distinct phases:

**CollectFactionIDs** (has side effects):
- Iterates `GetFactionInfo(1..GetNumFactions())` or `C_Reputation.GetFactionDataByIndex(1..C_Reputation.GetNumFactions())`, whichever the client exposes. These two calls only drive iteration; they are never themselves recorded to the trace.
- Expands collapsed headers to discover children.
- Returns ordered array of factionIDs.
- Updates `FactionOrder` if the set changed.

**SampleReputation** (pure reads):
- Samples every `FactionOrder` faction, i.e. the player's own reputation panel. Factions outside the panel are not recorded: a faction joins the panel on its first reputation change, so one outside it still has its starting value. Replay answers those from a static per-faction-group table (on Forever 1.60.1 the starting values outside the panel matched across the races checked, except one faction for each Skyborne race), which spares every player's client hundreds of lookups at login.
- A nil answer (a faction the client does not have) is recorded once, like any other value. Failed calls are skipped.
- Privacy: the guild faction (1168, Cataclysm and later clients) is named after the player's guild, so it is never sampled. Its ID stays in `FactionOrder`.
- For each sampled factionID, calls every available API independently: `C_Reputation.GetFactionDataByID(factionID)` when `C_Reputation` exists, and the legacy `GetFactionInfoByID(factionID)` when that global exists. Each result is recorded under its own API name (`C_Reputation.GetFactionDataByID[factionID]` / `GetFactionInfoByID[factionID]`) with its own change-detection state. A client exposing both gets both streams recorded; there is no synthesized composite value or exclusive either/or selection.
- Compares the raw value against the previous value (DeepCompare).
- Appends `{t, tp, v}` only if changed.

### Event-to-action mapping

| Event | Action |
|---|---|
| Capture start (Init) | CollectAndSample (both phases) |
| `QUEST_TURNED_IN` | CollectAndSample (quest rewards can reveal new factions) |
| `CHAT_MSG_COMBAT_FACTION_CHANGE` | CollectAndSample (a kill can reveal a new faction) |
| `UPDATE_FACTION` | CollectAndSample (any other reputation change) |
| `PLAYER_ENTERING_WORLD` | SampleReputation only (login-time sampling) |
| `SPELLS_CHANGED` | SampleReputation only (login-time sampling) |

Re-collecting walks the panel rows once. Headers are already expanded after the first pass, so later passes expand nothing and fire no further `UPDATE_FACTION`.

### Expand-only policy

After our first collection pass, all faction headers are expanded.
ExpandFactionHeader is called on collapsed headers but headers are never
re-collapsed. This is intentional — it simplifies the collection logic.

### Recursion guard

`ExpandFactionHeader` fires `UPDATE_FACTION`, which would re-enter the
tracker. A `collecting` flag prevents re-entry:

```lua
local collecting = false

function CollectFactionIDs()
  collecting = true
  -- ... iterate and expand ...
  collecting = false
end

OnEvent = function(capture, event, ...)
  if collecting then return end
  -- ...
end
```

### No delayed re-samples

Unlike quest functions, reputation changes are atomic — `GetFactionInfoByID`/`C_Reputation.GetFactionDataByID`
returns the correct value immediately when the event fires. No staggered
re-sampling is needed.

---

## 10) QuestLog Questie replay streams

The QuestLog tracker retains the existing active quest-log streams and adds Questie-oriented replay streams. `GetQuestLogQuestText[questId]` remains captured alongside current quest dialog text streams; they are different APIs.

Raw questID API streams preserve observed API behavior. Values for `IsQuestComplete`, `HaveQuestData`, `C_QuestLog.IsOnQuest`, `C_QuestLog.IsQuestFlaggedCompleted`, `C_QuestLog.GetQuestObjectives`, `GetQuestTagInfo`, reward count/money, and `GetQuestLogRewardInfo` are appended only after successful calls to those functions with the represented quest ID/arguments. When a quest leaves `QuestLog`, the tracker runs post-invalidation probes immediately and at the standard delayed offsets so traces capture the exact post-removal API behavior after Blizzard state settles. `C_QuestLog.IsQuestFlaggedCompleted` is related to `GetQuestsCompleted`, but the raw function stream is not derived from the completed-quest delta set.

Quest-log title/tag/completion data is split into several independent raw streams rather than one normalized composite value: `C_QuestLog.GetInfo[qid]`, `C_QuestLog.GetQuestTagInfo[qid]`, `C_QuestLog.IsComplete[qid]`, `C_QuestLog.IsFailed[qid]`, and the legacy `GetQuestLogTitle[qid]` (raw 17-value tuple, only recorded when that global exists). Each is probed and recorded independently, only when its own real API exists; a client lacking one of the modern `C_QuestLog.*` APIs simply produces no entries for that stream, it is never backfilled from another API. Likewise the quest-log index used to drive these lookups is recorded under whichever real API resolved it: `C_QuestLog.GetLogIndexForQuestID[qid]` or the legacy `GetQuestLogIndexByID[qid]`. Internally, the tracker also uses a private (unrecorded) row-reading helper purely to enumerate which quest-log indices hold quests vs. headers — that internal lookup is never itself written to the trace.

Timer streams are derived compatibility streams. The native Classic API exposes `GetQuestTimers()` as timer slots, then `GetQuestIndexForTimer(timerIndex)` maps a slot to a quest-log index. The tracker resolves that index to a quest ID and records both `GetQuestTimers[questId]` and `GetQuestLogTimeLeft[questId]` as seconds-left values. When a previously timed quest disappears from the timer mapping, both streams receive nil entries because the derived mapping is no longer valid; those nils are not raw native API return values. The slot mapping itself is also recorded raw as `GetQuestIndexForTimer[timerIndex]` (schema 15+), change-only, for the current slots only. Slots past the current timers are not read, because what the client returns for them is unverified. The current slot count is the number of quests with a non-nil derived `GetQuestTimers[questId]`. Together with the derived seconds and the quest-log index streams, this rebuilds `GetQuestTimers()` in slot order. A raw `GetQuestTimers()` stream is not recorded, because that key already holds the derived questID map.

Reward streams are quest-scoped except `GetQuestLogRewardInfo`, which is a true two-argument API and is stored in native argument order as `functions["GetQuestLogRewardInfo"][rewardIndex][questId]`. Previously observed reward indices continue to be probed during post-invalidation checks; failed calls are skipped rather than replaced with invented inactive values.

`GetQuestLogCompletionText` and `GetQuestLogSpecialItemInfo` take a quest-log index. Like `GetQuestLogQuestText`, they are sampled with the index resolved for each active quest and stored by quest ID, so the streams do not churn when other quests shift the log's indices. Replay resolves index → quest ID first. Completion text is passed through `Core.SanitizeText`. `C_QuestLog.GetRequiredMoney[questId]` is a direct quest-ID API, probed with the others, including after the quest leaves the log. `C_QuestLog.GetQuestTimers` is Forever's timer API and is recorded raw as a flat stream on every quest-log sample. The legacy `GetQuestTimers` global does not exist on Forever, so the derived timer streams below stay empty there.

`QuestLogZone[questId]` is a derived stream, not a raw API return value. While scanning quest-log rows to build the `QuestLog` quest-ID array, the tracker tracks the most recent header row's title (`isHeader` rows are excluded from `QuestLog` itself) and records it as the zone/category for every quest that follows until the next header. This lets replay consumers associate a quest with the zone heading Blizzard's quest log displays it under.

---

## 11) QuestDialog tracker behavior

The QuestDialog tracker captures transient gossip, greeting, and current quest dialog APIs used by replay consumers. It is separate from UnitInteraction because it records dialog state, not unit identity. It uses safe `pcall` wrappers and the standard delayed re-sample schedule for open/update events. Close events (`GOSSIP_CLOSED`, `QUEST_FINISHED`) cancel pending delayed reads and perform one observed API sample; raw streams do not receive deterministic synthetic inactive values.

Sampled parameterless streams include `C_GossipInfo.GetNumAvailableQuests`, `C_GossipInfo.GetNumActiveQuests`, `C_GossipInfo.GetText`, `C_GossipInfo.GetOptions`, legacy gossip count/list globals, greeting text/counts, current quest title/text/objective/progress/reward APIs, `GetRewardXP`, `IsQuestCompletable`, `GetNumQuestChoices`, `GetNumQuestRewards`, `GetNumQuestItems`, `GetNumQuestCurrencies`, `GetRewardMoney`, `GetQuestMoneyToGet`, `GetRewardTitle`, `GetRewardSkillPoints` (packed), `GetSuggestedGroupSize`, `QuestGetAutoAccept`, and `GetQuestBackgroundMaterial`.

Quest dialog items are stored in native argument order as `GetQuestItemInfo[type][index]` (packed tuple) and `GetQuestItemLink[type][index]` (scalar). Each sample pass reads `"choice"` indices 1..`GetNumQuestChoices()`, `"reward"` indices 1..`GetNumQuestRewards()` and `"required"` indices 1..`GetNumQuestItems()`. Indices beyond the current count are not read, since Blizzard's quest frame never asks for them. The delayed re-samples catch item names that arrive after the dialog event.

Reward currencies follow Mists' quest frame (`Wrath/QuestInfo.lua`, loaded for Wrath, Cata and Mists): `GetNumRewardCurrencies` and `GetMaxRewardCurrencies` are flat streams, and `GetQuestCurrencyInfo["reward"][index]` (packed) and `GetQuestCurrencyID["reward"][index]` are read for index 1..`GetMaxRewardCurrencies()`. That is past the count, because the frame reads every slot up to the maximum. Each API is feature-detected, so a client without these APIs records nothing; one that has them records them, whatever its flavor. Classic quest frames hardcode required currencies to 0, so they are not read; Forever's `C_QuestOffer` currency APIs are not recorded.

Greeting streams are keyed by greeting index: `GetActiveTitle[index]` and `GetAvailableQuestInfo[index]` as packed tuples, and `GetAvailableTitle[index]` and `GetActiveQuestID[index]` (Forever only) as scalars. Active and available indices each start at 1, the same as the native APIs, not offset like the greeting frame's button IDs. The highest count seen for each list (`GetNumActiveQuests` / `GetNumAvailableQuests`) is retained for the capture. When counts shrink, subsequent event and delayed samples continue probing stale indices so an initial error or unsettled return does not prevent later observations. Only successful returns are appended. A new capture resets these counts; close events still cancel delayed reads and perform only one sample.

Quest flags are keyed by quest ID: `IsBreadcrumbQuest[qid]`, `C_QuestLine.GetQuestLineInfo[qid]`, and `C_QuestInfoSystem.GetQuestClassification[qid]`. All three APIs are Forever-only. Each is feature-detected once at capture start, and when none exist (Era) no flags are sampled and neither `GetQuestID` nor the gossip list is read for this. Every sample pass (events and delayed re-samples) collects the dialog quest and the offered quests: `GetQuestID()` (the detail/progress/complete quest; 0 is ignored), each `questID` in `C_GossipInfo.GetAvailableQuests()`, and each questID (5th return) from `GetAvailableQuestInfo`. Active quests listed in gossip or greeting menus are not sampled unless they come up as the dialog quest. The tracker then calls each available flag API with that quest ID as its only argument. These reads only decide which quests are sampled. `GetQuestID` is recorded as its own flat stream and the gossip list is recorded by UnitInteraction. Each call is wrapped in `pcall`, and only successful returns are appended, change-only per quest. A gossip or greeting list holds only a few quests, so each pass makes at most about three calls per listed quest. `IsBreadcrumbQuest` is undocumented and no Blizzard UI code calls it, so passing the quest ID is an assumption and its value should be treated as a hint. `C_QuestLine.GetQuestLineInfo` is called without `uiMapID`/`displayableOnly`, so hidden quest lines are returned with `isHidden = true` rather than filtered out.

Privacy: every string these APIs return goes through `Core.SanitizeText` before it is stored, as AGENTS.md requires for free text from WoW APIs. This applies to `C_GossipInfo.GetText`, `GetGreetingText`, `GetQuestText`, `GetObjectiveText`, `GetProgressText`, `GetRewardText`, and `GetRewardTitle` (a character title can be formatted around the player's name). All seven are listed in `SANITIZE_TEXT_FUNCTIONS`. Sanitization replaces each top-level string of a scalar, packed tuple or returned table, and stores a copy so the API's own table is never modified. Quest titles (`GetActiveTitle`, `GetAvailableTitle`) and quest line names (`C_QuestLine.GetQuestLineInfo` `questLineName`/`questName`) are static game data and do not contain player names, so they are stored raw. The other streams hold only quest IDs, booleans and enum numbers. `QuestLineInfo` is otherwise stored whole rather than as a hand-picked subset. Its non-string fields are quest-start map data (`x`, `y`, `startMapID`, `floorLocation`) and quest/account state flags, none of which identify a player. The field set keeps growing across builds (`isQuestStart`, `inProgress`, `isAccountCompleted`, ...), and a whitelist would silently drop new fields that relationship inference could use.

---

## 12) SkillLines and SpellBook stale-index behavior

Skill and spellbook indices are mutable index spaces. When a previously observed
skill/profession index or spellbook slot is no longer reached by the current
count/enumeration, the trackers probe the old index with the represented raw API
and append only successful observed returns. They do not synthesize nil entries
for stale indices. `SpellBook` and `PlayerKnownSpells` remain explicit
synthetic/delta streams derived from observed spellbook enumeration.

---

## 13) ResetTime tracker behavior

The ResetTime tracker samples `GetServerTime` and `GetQuestResetTime` at capture start and on `PLAYER_LOGIN`, `PLAYER_ENTERING_WORLD`, and `PLAYER_LOGOUT`. These are low-frequency snapshots; replay consumers that need continuously increasing server time or countdown behavior should derive those values from the nearest snapshot and replay time.

---

## 14) Nameplate units (UnitInteraction)

On `NAME_PLATE_UNIT_ADDED(token)`, UnitInteraction records `UnitGUID[token]` for the new nameplate's unit. It samples only that token, and only tokens matching `nameplate%d+`; the event does not re-sample the fixed tokens. Only the GUID is recorded: Questie reads it for the NPC ID and only checks that `UnitName` is not nil, so replay answers the name from the NPC ID and no nameplate name is ever stored. Instances are sampled too, because Questie draws nameplate icons in dungeons on Classic clients. Forever restricts GUIDs there, and a restricted GUID records nil like any withheld unit.

Privacy: the GUID is classified with `Core.ParseGUIDKind`. Only `Creature-` and `Vehicle-` GUIDs of kind `npc` are recorded, the units Questie draws icons on. Players, game objects, unrecognized or restricted GUIDs, player pets and minions (`Pet-` GUIDs, or `UnitPlayerControlled(token)` true or restricted), and failed `UnitGUID` calls are withheld.

The stream is change-only per token, and a withheld unit's add records nil: "no recordable unit on this token", not a nil returned by the client. Nameplate tokens are reused as plates come and go; without that nil, a token taken over by a player would keep answering with its previous NPC. `NAME_PLATE_UNIT_REMOVED` writes no entry, so between a removal and the next add the token keeps its last value.

---

## 15) ItemCount tracker behavior

`C_Item.GetItemCount[itemID]` answers Questie's item objective and tooltip counts. Forever's `C_QuestLog.GetQuestObjectives` carries no item IDs, so the objective items cannot be picked out of the quest log. Instead the tracker reads the items the player carries: the backpack, bags 1–4, the keyring (when the client has one) and the equipment slots 0–19, since `GetItemCount` counts equipped items too. The bag and equipment reads only choose which items to sample and are not recorded.

- Capture start samples every carried item. An item without a `t=0` entry was not carried and had count 0.
- `ITEM_COUNT_CHANGED(itemID)` samples that item immediately. Forever skips this event for some pickups (often an item's first), and Era does not have it, so it is not enough on its own.
- `BAG_UPDATE_DELAYED`, on a private frame that records nothing, schedules one rescan a second later; further bag updates in that second share it. The rescan reads every carried item plus every recorded item whose last count was above 0, so an item that left the bags is recorded as 0. Items at 0 that are not carried are not read again.

All samples are change-only. A full rescan measured 0.21 ms on Forever with 20 bag slots and 25 items, and runs at most once per second, only after bag changes.

---

## 16) Display tracker behavior

The Display tracker records the screen geometry (`GetPhysicalScreenSize`, `UIParent:GetScale`, `GetScreenWidth`, `GetScreenHeight`) and `GetCVar[name]` for the CVars in `RECORDED_CVARS` (`Modules/Trackers/Display.lua`): `useUiScale`, `uiScale`, `questPOI`, `autoQuestWatch`, `autoQuestProgress`, `miniWorldMap`, `mapFade`, `rotateMinimap`, `minimapZoom`, `minimapInsideZoom`, `instantQuestText`, `displayQuestID` and `closedInfoFrames` (a bitfield string of the first-time help tips the player closed). All but `displayQuestID` exist on Forever 1.60.1; it is kept because Questie reads it and other clients may have it. A CVar a client lacks records nil. CVar streams are keyed by the lowercase name, since CVar names are case-insensitive and callers use both spellings. It samples at capture start and on `PLAYER_ENTERING_WORLD`, `UI_SCALE_CHANGED`, `DISPLAY_SIZE_CHANGED` and `CVAR_UPDATE`, change-only. A nil CVar value is recorded once.

Privacy: `CVAR_UPDATE` fires for every CVar, and some CVars hold account or realm identifiers. Its first argument is the CVar name or an event name passed to `SetCVar` (the world map sends `"QUEST_POI"` for `questPOI`). The tracker therefore listens on a private frame: any `CVAR_UPDATE` re-reads the allowlisted CVars, and the event itself is never recorded or dispatched.

---

## 17) ItemInfo tracker behavior

The ItemInfo tracker measures how late item data arrives. At the first capture start it post-hooks the item data requests with `hooksecurefunc`: `C_Item.GetItemInfo`, the `GetItemInfo` global on clients that have it, and `C_Item.RequestLoadItemDataByID`, which Blizzard's `Item:ContinueOnItemLoad` uses. The hooks see the calls of every addon that looks these functions up when calling. Questie does since v12.0.4, through `QuestieCompat.GetItemInfo`. Addons that saved a function in a local before the hooks were installed are not seen. Hooks cannot be removed, so they stay installed and do nothing while no capture runs.

A post-hook sees the arguments but not the answer, so the hook asks `C_Item.IsItemDataCachedByID` instead. The original call has already started loading an uncached item, so a `false` there is the moment an addon asked for data the client did not have. The hook reduces the argument to an item ID: a whole number from 1 to 2^31−1, given as a number, a numeric string or the ID in an item link. Item names resolve only for cached items and start no load, so they are skipped. An item found cached, or given a stream, is not checked again during the capture.

`functions["C_Item.IsItemDataCachedByID"][itemID]` then holds `false` at that first request, and `true` once the data arrives. The arrival event follows the request: in the Forever 1.60.1 client a `GetItemInfo` load fired only `GET_ITEM_INFO_RECEIVED`, and a `RequestLoadItemDataByID` load only `ITEM_DATA_LOAD_RESULT`. Both fire for every item any addon or tooltip loads, so they are private triggers: only items with a stream are re-read, and the events are never recorded.

Auction house and collection addons ask about thousands of items, so the streams are capped in two steps. The first 500 come from any addon's requests. Past 500, a request gets a stream only when `debugstack` shows Questie's folder (`AddOns/Questie/`) in the caller's frame or the three above it, up to 1000 streams. That window covers Questie's `QuestieCompat.GetItemInfo` wrapper and Blizzard's `Item:ContinueOnItemLoad` called from Questie. Another addon's refused request leaves the item unchecked, so Questie asking for it while it still loads gets a stream. After three refused requests the item is given up, so an addon polling an item that never loads cannot read the stack all capture. The stack is read only for uncached requests past 500 and before 1000 streams, and a session with 500 streams or more reached that switch.

Cost: in the Forever 1.60.1 client the hook body took about 0.5 µs per call with an item ID and 1.5 µs with an item link, against 9.7–13 µs for `C_Item.GetItemInfo` on a cached item. Reading the four frames took about 10 µs (about 2.5 µs per frame). An auction scan that finds 8,000 items uncached past 500 streams therefore costs about 0.1 s in total with one request per item, and at most about 0.25 s, spread over the scan; items that have loaded cost no stack read on later scans.

---

## 18) FrameHeartbeat tracker behavior

The FrameHeartbeat tracker shows a frame whose `OnUpdate` script runs on every client frame while a capture is active, and hides it when the capture stops. It writes `session.frameUpdates` (see SCHEMA_SPEC): the first 10 frame updates after capture start, then only frame updates that end a gap of more than one second, with `v` = seconds since the previous frame update. Capture starts at `PLAYER_LOGIN`, before the login loading screen ends, so the first entries show whether addon frames run during it. For later loading screens, a gap entry covers `[tp - v, tp]`; a loading screen of a second or less cannot be told apart. Each frame costs one `GetTimePreciseSec()` call and a comparison.

---

## 19) Change detection

Trackers only append entries when values change. The comparison method
depends on the value type:

| Value type | Comparison | Used by |
|---|---|---|
| Scalar (number, string, boolean) | `==` | UnitLevel, Position zone texts, loot scalars |
| Table/object | `DeepCompare()` | Reputation tuples, quest objectives, quest log membership |
| Nil | Explicit nil check | Raw API calls that actually returned nil |

`DeepCompare` performs recursive key-by-key comparison with cycle
detection and optional metatable comparison.
