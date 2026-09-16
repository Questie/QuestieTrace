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

There is no serialization step on save. The session object built by
trackers IS the SessionRecord stored in SavedVariables.

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
2. Session is appended directly to `QuestieTraceCharacter.sessions`.
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
| Position | `Trackers/Position.lua` | `GetZoneText`, `GetSubZoneText`, `GetRealZoneText`, `C_Map.GetBestMapForUnit["player"]`, `C_Map.GetPlayerMapPosition["player"]`, `IsInInstance`, `GetInstanceInfo` | Timer (0.2s) + event-driven + private movement sampling | zone/map events + `PLAYER_ENTERING_WORLD`, `PLAYER_ALIVE`, `SPELLS_CHANGED`; movement events are private sampling triggers and are not recorded globally |
| Loot | `Trackers/Loot.lua` | `GetNumLootItems`, `GetLootSlotInfo[slot]`, `GetLootSourceInfo[slot]`, `GetLootSlotLink[slot]`, `GetLootSlotType[slot]` | Event + window lifecycle | `LOOT_READY`, `LOOT_CLOSED` |
| Reputation | `Trackers/Reputation.lua` | `FactionOrder`, `GetFactionInfoByID[factionID]` | Event + index iteration | `CHAT_MSG_COMBAT_FACTION_CHANGE`, `UPDATE_FACTION`, `QUEST_TURNED_IN`, `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| QuestLog | `Trackers/QuestLog.lua` | `QuestLog`, `C_QuestLog.GetMaxNumQuestsCanAccept`, `IsQuestComplete[qid]`, `HaveQuestData[qid]`, `C_QuestLog.IsOnQuest[qid]`, `C_QuestLog.IsQuestFlaggedCompleted[qid]`, `C_QuestLog.GetQuestObjectives[qid]`, `GetQuestLogTitle[qid]`, `GetQuestLogQuestText[qid]`, timer streams, reward streams, `GetQuestTagInfo[qid]` | Event + delayed re-samples + index iteration | 14 quest events + `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| CompletedQuests | `Trackers/CompletedQuests.lua` | `GetQuestsCompleted` (functionsDelta) | Event + delayed re-samples | Same 14 quest events + `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| UnitInteraction | `Trackers/UnitInteraction.lua` | `UnitGUID`/`UnitName` for `target`, `npc`, `questnpc`, `mouseover`; `C_GossipInfo.GetAvailableQuests`; `C_GossipInfo.GetActiveQuests` | Event-driven fixed unit-token fanout | target, quest dialog, selected quest state, loot-open, NPC interaction, login-time events, `UPDATE_MOUSEOVER_UNIT` |
| GroupState | `Trackers/GroupState.lua` | `IsInGroup`, `GetNumGroupMembers` | Event-driven | `GROUP_JOINED`, `GROUP_LEFT`, `GROUP_ROSTER_UPDATE`, `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| SkillLines | `Trackers/SkillLines.lua` | `GetNumSkillLines`, `GetSkillLineInfo[index]`, `GetProfessions`, `GetProfessionInfo[index]` | Event + index iteration | `SKILL_LINES_CHANGED`, `PLAYER_ENTERING_WORLD`, `SPELLS_CHANGED` |
| SpellBook | `Trackers/SpellBook.lua` | `SpellBook`, `GetSpellBookItemName[slot]`, `GetSpellBookItemInfo[slot]`, `IsPassiveSpell[slot]`, `PlayerKnownSpells` (functionsDelta) | Event + slot iteration | `SPELLS_CHANGED`, `PLAYER_ENTERING_WORLD` |
| QuestDialog | `Trackers/QuestDialog.lua` | Gossip, greeting, and current quest-dialog APIs | Event + delayed re-samples + observed close sample | `QUEST_DETAIL`, `QUEST_PROGRESS`, `QUEST_COMPLETE`, `QUEST_FINISHED`, `QUEST_GREETING`, `QUEST_ACCEPT_CONFIRM`, `GOSSIP_SHOW`, `GOSSIP_CLOSED` |
| ResetTime | `Trackers/ResetTime.lua` | `GetServerTime`, `GetQuestResetTime` | Init + low-frequency event snapshots | `PLAYER_LOGIN`, `PLAYER_ENTERING_WORLD`, `PLAYER_LOGOUT` |

---

## 8) Reputation tracker behavior

The Reputation tracker has unique complexity that warrants explicit
documentation.

### Collection vs sampling

Two distinct phases:

**CollectFactionIDs** (has side effects):
- Iterates `GetFactionInfo(1..GetNumFactions())`.
- Expands collapsed headers to discover children.
- Returns ordered array of factionIDs.
- Updates `FactionOrder` if the set changed.

**SampleReputation** (pure reads):
- For each known factionID, calls `GetFactionInfoByID(factionID)`.
- Compares full 16-value tuple against previous value (DeepCompare).
- Appends `{t, tp, v}` only if changed.

### Event-to-action mapping

| Event | Action |
|---|---|
| Capture start (Init) | CollectAndSample (both phases) |
| `QUEST_TURNED_IN` | CollectAndSample (quest rewards can reveal new factions) |
| `CHAT_MSG_COMBAT_FACTION_CHANGE` | SampleReputation only (values changed, set unchanged) |
| `UPDATE_FACTION` | SampleReputation only (same) |
| `PLAYER_ENTERING_WORLD` | SampleReputation only (login-time sampling) |
| `SPELLS_CHANGED` | SampleReputation only (login-time sampling) |

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

Unlike quest functions, reputation changes are atomic — `GetFactionInfoByID`
returns the correct value immediately when the event fires. No staggered
re-sampling is needed.

---

## 10) QuestLog Questie replay streams

The QuestLog tracker retains the existing active quest-log streams and adds Questie-oriented replay streams. `GetQuestLogQuestText[questId]` remains captured alongside current quest dialog text streams; they are different APIs.

Raw questID API streams preserve observed API behavior. Values for `IsQuestComplete`, `HaveQuestData`, `C_QuestLog.IsOnQuest`, `C_QuestLog.IsQuestFlaggedCompleted`, `C_QuestLog.GetQuestObjectives`, `GetQuestTagInfo`, reward count/money, and `GetQuestLogRewardInfo` are appended only after successful calls to those functions with the represented quest ID/arguments. When a quest leaves `QuestLog`, the tracker runs post-invalidation probes immediately and at the standard delayed offsets so traces capture the exact post-removal API behavior after Blizzard state settles. `C_QuestLog.IsQuestFlaggedCompleted` is related to `GetQuestsCompleted`, but the raw function stream is not derived from the completed-quest delta set.

Timer streams are derived compatibility streams. The native Classic API exposes `GetQuestTimers()` as timer slots, then `GetQuestIndexForTimer(timerIndex)` maps a slot to a quest-log index. The tracker resolves that index to a quest ID and records both `GetQuestTimers[questId]` and `GetQuestLogTimeLeft[questId]` as seconds-left values. When a previously timed quest disappears from the timer mapping, both streams receive nil entries because the derived mapping is no longer valid; those nils are not raw native API return values.

Reward streams are quest-scoped except `GetQuestLogRewardInfo`, which is a true two-argument API and is stored in native argument order as `functions["GetQuestLogRewardInfo"][rewardIndex][questId]`. Previously observed reward indices continue to be probed during post-invalidation checks; failed calls are skipped rather than replaced with invented inactive values.

---

## 11) QuestDialog tracker behavior

The QuestDialog tracker captures transient gossip, greeting, and current quest dialog APIs used by replay consumers. It is separate from UnitInteraction because it records dialog state, not unit identity. It uses safe `pcall` wrappers and the standard delayed re-sample schedule for open/update events. Close events (`GOSSIP_CLOSED`, `QUEST_FINISHED`) cancel pending delayed reads and perform one observed API sample; raw streams do not receive deterministic synthetic inactive values.

Sampled parameterless streams include `C_GossipInfo.GetNumAvailableQuests`, `C_GossipInfo.GetNumActiveQuests`, `C_GossipInfo.GetText`, `C_GossipInfo.GetOptions`, legacy gossip count/list globals, greeting text/counts, current quest title/text/objective/progress/reward APIs, `GetRewardXP`, `IsQuestCompletable`, and `GetNumQuestChoices`.

Indexed streams are `GetActiveTitle[index]` as a packed tuple and `GetAvailableTitle[index]` as a scalar title. The highest count seen for each API is retained for the capture. When counts shrink, subsequent event and delayed samples continue probing stale indices so an initial error or unsettled return does not prevent later observations. Only successful returns are appended. A new capture resets these counts; close events still cancel delayed reads and perform only one sample.

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

## 14) Change detection

Trackers only append entries when values change. The comparison method
depends on the value type:

| Value type | Comparison | Used by |
|---|---|---|
| Scalar (number, string, boolean) | `==` | UnitLevel, Position zone texts, loot scalars |
| Table/object | `DeepCompare()` | Reputation tuples, quest objectives, quest log membership |
| Nil | Explicit nil check | Raw API calls that actually returned nil |

`DeepCompare` performs recursive key-by-key comparison with cycle
detection and optional metatable comparison.
