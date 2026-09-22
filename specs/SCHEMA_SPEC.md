# Schema Spec (v9)

## 1) SavedVariables

### `QuestieTrace` (account-level)

```lua
QuestieTrace = {
  schemaVersion = 9,
  settings = {
    autoStart = true,
  },
}
```

### `QuestieTraceDumps` (account-level)

```lua
QuestieTraceDumps = {
  schemaVersion = 1,
  dumps = {
    map_hierarchy = MapHierarchyDumpData,
  },
}
```

### `QuestieTraceCharacter` (per-character)

```lua
QuestieTraceCharacter = {
  lastSavedSession = "2026-02-10_12-34-56",
  currentSession = SessionRecord?,          -- live/stopped-unsaved session, linked by reference
  sessions = { SessionRecord, ... },
  savedSessionCounter = 0,                  -- monotonic count of sessions ever saved
  reminder = {
    sessionCounterAtExport = 0,             -- savedSessionCounter when the export window was last opened
  },
}
```

`currentSession` is a direct reference to the in-memory `capture.session` table established by `Core.StartCapture()`. Since trackers mutate the table in place,
no periodic sync is needed — the reference remains valid for the session's lifetime. It is cleared by `Core.SaveCapture()` (session moved to `sessions[]`).
On `VARIABLES_LOADED`, if a leftover `currentSession` exists, it is recovered as a stopped-unsaved session (see Bootstrap sequence).

`lastSavedSession` is only set on explicit save — it is not initialized
on fresh install or migration.

`savedSessionCounter` increments on every `Core.SaveCapture()` and never decreases.
`#sessions` shrinks whenever a reported session is deleted by
`Core.DeleteReportedSessions()`, so it cannot be used as a "has new data been
saved?" watermark; the share reminder compares this counter against
`reminder.sessionCounterAtExport` instead (see `specs/UI_SPEC.md` section 5).

### Migration

If `schemaVersion ~= 9`, the account-level table is wiped and recreated
with defaults. Per-character sessions from older versions are lost. There
is no incremental migration from v8 to v9.

### Session lifetime

A session lives in `QuestieTraceCharacter.sessions` until the player confirms
they reported its data via the "I reported this" button in the export
window, at which point `Core.DeleteReportedSessions()` removes it
permanently (see "Export and deletion" below). This keeps SavedVariables
small in the common case without an arbitrary cap on unreported data.

As a pure size backstop for players who never open the export window,
`Core.SaveCapture()` also prunes the oldest saved session(s) whenever
`#sessions` exceeds `MAX_SESSIONS` (10), regardless of report status — see
"Session pruning" in `specs/ARCHITECTURE_SPEC.md`.

### Export and deletion

`Core.BuildExportPayload()` returns every saved session (and the live
`currentSession`, if it has events) — there is no "already exported" state to
skip, since reported sessions are deleted rather than flagged.
`Core.DeleteReportedSessions(sourceSessions)` is called only once the player
explicitly confirms via "I reported this" (not merely on showing the window):
it removes matching entries from `QuestieTraceCharacter.sessions` and, if the
live session was included, discards it (without saving) and starts a fresh
capture so future events land in a new, distinct session.

This is a coarse, best-effort proxy — the addon cannot know whether a shown
export string was actually copied and submitted, only that the player
clicked the button confirming they did.

---

## 2) SessionRecord

Each `/qlt save` appends one record to `QuestieTraceCharacter.sessions`.

```lua
SessionRecord = {
  schemaVersion = 9,
  recordingContractVersion = 1,
  name = "2026-02-10_12-34-56",

  startedAt        = 100000.000,     -- GetTime() at capture start
  startedAtPrecise = 4821.31204,     -- GetTimePreciseSec() at capture start
  stoppedAt        = 100333.150,     -- GetTime() at capture stop
  stoppedAtPrecise = 5154.46238,     -- GetTimePreciseSec() at capture stop
  duration         = 333.150,        -- stoppedAt - startedAt
  durationPrecise  = 333.15034,      -- stoppedAtPrecise - startedAtPrecise

  events         = EventEntry[],
  functions      = table<string, FunctionStream>,
  functionsDelta = table<string, DeltaStream>,
}
```

There is no `exportedAt` field: a reported session is deleted outright by
`Core.DeleteReportedSessions()` rather than flagged and kept around (see
"Export and deletion" above).

### Recording contract

`recordingContractVersion` identifies recording semantics independently of the
storage schema. New captures use `1`: raw API streams contain successful API
observations, not synthetic close/removal resets. Explicit synthetic/derived
streams retain their documented semantics. Failed calls produce no sample; the
last observation is not proof that an API still returns that value later.

Missing markers mean legacy/unknown semantics, even for schema v9. Those sessions
may contain synthetic resets or normalized returns. Consumers must check for the
supported value `1` before applying the observed-only guarantee; other versions
are unknown until supported. Do not infer or backfill a marker from stream data.
Existing saves and settings are unchanged; this field requires no schema migration.

No `summary` block — consumers derive counts from the data.
No `player` block — player identity is stored as function streams
(`UnitRace`, `UnitClass`, `UnitClassBase`, `UnitSex`, `UnitFactionGroup`).

---

## 3) Time model

All timestamps in entries are **session-relative** (seconds since capture
start). Two clocks are stored on every entry:

- `t` — from `GetTime()`. Cached once per frame. All samples in the same
  frame share the same `t`.
- `tp` — from `GetTimePreciseSec()`. Monotonic, millisecond precision,
  unique per call.

Conversion from absolute to relative at capture time:

```lua
t  = GetTime()          - session.startedAt
tp = GetTimePreciseSec() - session.startedAtPrecise
```

The session envelope stores absolute baselines so consumers can convert
back if needed.

---

## 4) Events

```lua
{ t = 0.000, tp = 0.00012, e = "QUEST_ACCEPTED", a = { 101, 56789, n = 2 } }
```

- `t`: session-relative `GetTime()`
- `tp`: session-relative `GetTimePreciseSec()`
- `e`: event name string
- `a`: packed args (see section 7)

---

## 5) Function streams

All tracked WoW API return values are stored in `functions`. Streams are
either flat arrays for parameterless APIs or argument-keyed maps for
parameterized APIs. True multi-argument APIs use nested argument-keyed maps in
native API argument order until the final value is a `{t, tp, v}` array.

### Parameterless — flat array

```lua
["GetZoneText"] = {
  { t = 0.000, tp = 0.00015, v = "Dun Morogh" },
  { t = 90.000, tp = 90.00022, v = "Stormwind City" },
}
```

### Parameterized — keyed by argument

```lua
["UnitLevel"] = {
  ["player"] = {
    { t = 0.000, tp = 0.00020, v = 12 },
    { t = 5.213, tp = 5.21350, v = 13 },
  },
}
```

### Nested parameterized — keyed by native argument order

```lua
["GetQuestLogRewardInfo"] = {
  [1] = { -- rewardIndex
    [783] = { -- questId
      { t = 0.000, tp = 0.00020, v = { "Reward", 134400, 1, 1, true, 12345, 10, n = 7 } },
    },
  },
}
```

### Parser detection

If the first entry in a table has a `t` field, that table is a leaf stream
array. Otherwise, keys are argument values and consumers should recurse until a
leaf stream array is reached. The TypeScript analyzer normalizes Lua tables
first, then treats arrays as leaf streams and objects as parameter maps.

### Sampling cadence and change detection

Most streams append entries only when values change. Streams known at capture
start include an initial value at `t=0`; parameterized streams for later
discovered keys, such as newly accepted quest IDs, can begin after `t=0`. The
latest entry before a target time is the current recorded value.

### `nil` is a valid value

When a function returns `nil`, that is stored as a change. Lua serialization
omits `v = nil`, so a saved entry with no `v` field means the function returned
nil at that timestamp. Consumers MUST treat a missing `v` field as a stored nil
value, not as missing data.

---

## 6) Delta streams

For functions that return large sets (e.g. completed-quest IDs —
thousands of quest IDs). Stored in `functionsDelta`.

`C_QuestLog.GetAllCompletedQuestIDs` and the legacy `GetQuestsCompleted`
global are tracked as two fully independent delta streams, each only
present when its own real API exists on the capturing client. There is no
merged/normalized value between them.

```lua
["C_QuestLog.GetAllCompletedQuestIDs"] = {
  t = 0,
  tp = 0,
  initial = { 123, 456, 789 },
  delta = {
    { t = 50.000, tp = 50.00012, add = { 56789 } },
    { t = 120.000, tp = 120.00034, add = { 67890, 11111 } },
  },
}
```

- `initial`: full set at capture start.
- `delta`: ordered entries with `add` and/or `remove` arrays.
- Empty `add`/`remove` arrays are omitted during serialization.
- Completed-quest sets only ever grow in practice (quests cannot be
  uncompleted), but the format supports `remove` for generality.

---

## 7) Packed args encoding

Used in event args and tuple-returning function values.

```lua
{ value1, value2, ..., n = argCount }
```

`n` preserves the argument count even when `nil` appears in the middle.

**Sparse arrays after serialization.** When WoW API functions return `nil`
in middle positions, Lua serialization omits those values, creating gaps
in the stored array (e.g. indices jump from 3 to 5). The `n` field is
authoritative for the true argument count. Consumers must use
`unpack(v, 1, v.n)` to correctly reconstruct nils for missing indices.
This is expected behavior, not a bug.

---

## 8) Return value format

The stored `v` in a function entry is self-describing:

| `v` shape | Meaning | Emulator action |
|---|---|---|
| `type(v) == "table" and v.n` | Packed tuple | `return unpack(v, 1, v.n)` |
| `type(v) == "table"` (no `n`) | Object/table | `return v` |
| scalar (number, string, boolean, nil) | Single value | `return v` |

All tuple-returning functions MUST have `n` on every stored value.

---

## 9) Complete function catalog

> **API vs synthetic/derived keys.** Most keys are direct WoW API names.
> For sessions with `recordingContractVersion = 1`, raw API stream values come
> from successful calls to that API with the represented arguments. Event-driven
> close/removal/count-shrink sampling records whatever the API actually returns;
> it does not invent inactive values.
> Synthetic keys are computed by trackers. Derived compatibility streams use a
> WoW API-like name but store a replay-friendly shape.

### Parameterless

| Function key | Return type | Notes |
|---|---|---|
| `GetZoneText` | scalar (string) | |
| `GetSubZoneText` | scalar (string) | |
| `GetRealZoneText` | scalar (string) | |
| `IsInInstance` | tuple (n=2) | inInstance, instanceType |
| `GetInstanceInfo` | tuple (n=10) | name, instanceType, difficultyID, ... |
| `GetNumLootItems` | scalar (number) | 0 when no loot window |
| `IsInGroup` | scalar (boolean) | |
| `GetNumGroupMembers` | scalar (number) | 0 when not in a group |
| `GetNumSkillLines` | scalar (number) | Legacy global; only present when it exists, no synthesized fallback |
| `GetQuestGreenRange` | scalar (number) | XP threshold; changes with player level |
| `GetProfessions` | tuple (n=5) | profession tab indices; nil in missing tuple slots |
| `C_TradeSkillUI.GetAllProfessionTradeSkillLines` | object (number[]) | Tracked independently whenever the API exists |
| `C_GossipInfo.GetAvailableQuests` | object (GossipQuestUIInfo[]) | UnitInteraction; sampled on gossip/dialog events |
| `C_GossipInfo.GetActiveQuests` | object (GossipQuestUIInfo[]) | UnitInteraction; sampled on gossip/dialog events |
| `C_GossipInfo.GetNumAvailableQuests` | scalar (number) | QuestDialog; when API exists |
| `C_GossipInfo.GetNumActiveQuests` | scalar (number) | QuestDialog; when API exists |
| `C_GossipInfo.GetText` | scalar (string/nil) | QuestDialog; when API exists |
| `C_GossipInfo.GetOptions` | object (GossipOptionUIInfo[]) | QuestDialog; when API exists |
| `GetNumGossipAvailableQuests` | scalar (number) | Legacy gossip API |
| `GetNumGossipActiveQuests` | scalar (number) | Legacy gossip API |
| `GetGossipAvailableQuests` | tuple (n varies) | Legacy raw repeated 7-tuples |
| `GetGossipActiveQuests` | tuple (n varies) | Legacy raw repeated 6-tuples |
| `GetGreetingText` | scalar (string/nil) | Quest greeting text |
| `GetNumActiveQuests` | scalar (number) | Quest greeting active count |
| `GetNumAvailableQuests` | scalar (number) | Quest greeting available count |
| `GetQuestID` | scalar (number) | Current quest dialog ID; often 0 outside dialog |
| `GetTitleText` | scalar (string/nil) | Current quest dialog title |
| `GetQuestText` | scalar (string/nil) | Current quest detail text |
| `GetObjectiveText` | scalar (string/nil) | Current objective text |
| `GetProgressText` | scalar (string/nil) | Current progress text |
| `GetRewardText` | scalar (string/nil) | Current reward text |
| `GetRewardXP` | scalar (number/nil) | Current quest reward XP |
| `IsQuestCompletable` | scalar (boolean/nil) | Current quest progress state |
| `GetNumQuestChoices` | scalar (number) | Current reward choice count |
| `C_QuestLog.GetMaxNumQuestsCanAccept` | scalar (number) | Captured when API exists |
| `GetServerTime` | scalar (number) | Low-frequency snapshot |
| `GetQuestResetTime` | scalar (number) | Legacy global; only present when it exists |
| `C_DateAndTime.GetSecondsUntilDailyReset` | scalar (number) | Only present when this API exists |
| `QuestLog` | object (number[]) | *(synthetic)* active quest IDs in quest-log order |
| `FactionOrder` | object (number[]) | *(synthetic)* known faction IDs in display order |
| `SpellBook` | object (number[]) | *(synthetic)* ordered unique spell IDs from slot enumeration |

### Parameterized by `"player"`

| Function key | Return type | Notes |
|---|---|---|
| `UnitLevel` | scalar (number) | |
| `UnitRace` | tuple (n=3) | localizedName, englishName, raceID |
| `UnitClass` | tuple (n=3) | localizedName, englishName, classID |
| `UnitClassBase` | tuple (n=2) | classFilename, classID; captured when API exists |
| `UnitSex` | scalar (number) | |
| `UnitFactionGroup` | tuple (n=2) | englishFaction, localizedFaction |
| `C_Map.GetBestMapForUnit` | scalar (number) | map ID |
| `C_Map.GetPlayerMapPosition` | object ({x, y})/nil | *(derived compatibility)* current-map coordinates rounded to 4 decimals; nil when map/position is unavailable, including when no position API call was possible |

### Parameterized by questId

| Function key | Return type | Notes |
|---|---|---|
| `IsQuestComplete` | scalar (boolean) | |
| `HaveQuestData` | scalar (boolean/nil) | Captured when API exists |
| `C_QuestLog.IsOnQuest` | scalar (boolean/nil) | Raw API result; probed again after quest leaves log |
| `C_QuestLog.IsQuestFlaggedCompleted` | scalar (boolean/nil) | Raw API result; related to but not derived from `GetQuestsCompleted` |
| `C_QuestLog.GetQuestObjectives` | object (QuestObjectiveInfo[]/nil) | Raw API result; probed again after quest leaves log |
| `C_QuestLog.GetInfo` | object (table) | Raw `C_QuestLog.GetInfo` return, keyed by questID after resolving the current quest log index |
| `C_QuestLog.GetQuestTagInfo` | object (table/nil) | Raw API result |
| `C_QuestLog.IsComplete` | scalar (boolean/nil) | Raw API result |
| `C_QuestLog.IsFailed` | scalar (boolean/nil) | Raw API result |
| `C_QuestLog.GetLogIndexForQuestID` | scalar (number/nil) | Raw API result; used to resolve the quest-log index for the streams above |
| `GetQuestLogTitle` | tuple (n=17) | Legacy global; raw tuple, stored by quest ID after resolving current quest log index. Only present when this global exists -- not synthesized from `C_QuestLog.*` |
| `GetQuestLogIndexByID` | scalar (number/nil) | Legacy global; raw API result |
| `GetQuestLogQuestText` | tuple (n=2) | questDescription, questObjectives |
| `GetQuestTimers` | scalar (number/nil) | *(derived compatibility)* questId-keyed seconds-left from native timer slots |
| `GetQuestLogTimeLeft` | scalar (number/nil) | *(derived compatibility)* same seconds-left value, no selection side effects |
| `GetNumQuestLogRewards` | scalar (number/nil) | Raw API result; probed again after quest leaves log |
| `GetQuestLogRewardMoney` | scalar (number/nil) | Raw API result; probed again after quest leaves log |
| `GetQuestTagInfo` | tuple (n varies) | |

### Nested parameterized by native arguments

| Function key | Shape | Return type | Notes |
|---|---|---|---|
| `GetQuestLogRewardInfo` | `[rewardIndex][questId]` | tuple (n=7) or nil | Raw API result in native argument order; previously observed indices are probed again after counts shrink or quest leaves log |

### Parameterized by greeting index

| Function key | Return type | Notes |
|---|---|---|
| `GetActiveTitle` | tuple (n=2) | title, isComplete |
| `GetAvailableTitle` | scalar (string/nil) | title; stale indices are probed with the actual API when counts shrink |

### Parameterized by unit token (`"target"`, `"npc"`, `"questnpc"`)

| Function key | Return type | Notes |
|---|---|---|
| `UnitGUID` | scalar (string) or nil | GUID string; nil when no unit |
| `UnitName` | packed tuple (n varies) | observed `UnitName(token)` returns; no synthetic `UnitExists` mapping |

### Parameterized by slot index

| Function key | Return type | Notes |
|---|---|---|
| `GetLootSlotInfo` | tuple (n=9) | observed slot API return |
| `GetLootSourceInfo` | tuple (n=2) | observed slot API return |
| `GetLootSlotLink` | scalar (string/nil) | observed slot API return |
| `GetLootSlotType` | scalar (number/nil) | observed slot API return |
| `GetSpellBookItemName` | tuple (n varies) | observed spellbook name API return |
| `GetSpellBookItemInfo` | tuple (n varies) | observed spellbook info API return |
| `IsPassiveSpell` | scalar (number/nil) | observed passive marker return |

### Parameterized by factionID

| Function key | Return type | Notes |
|---|---|---|
| `GetFactionInfoByID` | tuple (n=16) | Legacy global; raw tuple, only present when the legacy global exists (independent of whether C_Reputation.GetFactionDataByID also exists) |
| `C_Reputation.GetFactionDataByID` | object (table) | Raw API result |

### Parameterized by skill index / profession tab index / trade-skill line ID

| Function key | Return type | Notes |
|---|---|---|
| `GetSkillLineInfo` | tuple (n=13) | Legacy global; keyed by visible skill-line row index after header expansion. Only present when `GetNumSkillLines` exists -- no synthesized fallback |
| `GetProfessionInfo` | tuple (n=10) | keyed by profession tab index returned by `GetProfessions()` |
| `C_TradeSkillUI.GetTradeSkillLineInfoByID` | object (table) | Raw API result, keyed by skill-line ID; tracked independently whenever the API exists |

### Delta streams (in `functionsDelta`)

| Function key | Notes |
|---|---|
| `C_QuestLog.GetAllCompletedQuestIDs` | Completed quest IDs; only grows in practice |
| `GetQuestsCompleted` | Legacy global; independent completed-quest-ID stream, tracked whenever this global exists (not only as a fallback) |
| `PlayerKnownSpells` | Known player spell IDs discovered by enumerating spellbook slots |

---

## 10) Complete annotated session example

Below is an unmarked legacy v9 `SessionRecord` illustrating the storage shapes.
It includes synthetic loot close resets, which must not be interpreted as native
API observations. New recordings carry `recordingContractVersion = 1` and store
only observed raw API returns instead of those resets.

```lua
{
  schemaVersion = 9,
  name = "2026-02-10_12-34-56",

  -- Session envelope: absolute clock baselines and derived durations
  startedAt        = 100000.000,     -- GetTime() at capture start
  startedAtPrecise = 4821.31204,     -- GetTimePreciseSec() at capture start
  stoppedAt        = 100333.150,     -- GetTime() at capture stop
  stoppedAtPrecise = 5154.46238,     -- GetTimePreciseSec() at capture stop
  duration         = 333.150,        -- stoppedAt - startedAt
  durationPrecise  = 333.15034,      -- stoppedAtPrecise - startedAtPrecise

  -----------------------------------------------------------------------
  -- Events: raw time-series of game signals
  -----------------------------------------------------------------------
  events = {
    { t = 0.000, tp = 0.00012, e = "PLAYER_ENTERING_WORLD", a = { n = 0 } },
    { t = 0.500, tp = 0.50021, e = "QUEST_ACCEPTED", a = { 101, 56789, n = 2 } },
    { t = 5.213, tp = 5.21347, e = "PLAYER_LEVEL_UP", a = { 13, 120, 40, 0, 0, 0, 1, 1, 1, 1, n = 10 } },
    { t = 45.200, tp = 45.20021, e = "LOOT_READY", a = { n = 0 } },
    { t = 48.100, tp = 48.10008, e = "LOOT_CLOSED", a = { n = 0 } },
    { t = 50.000, tp = 50.00005, e = "QUEST_TURNED_IN", a = { 56789, n = 1 } },
  },

  -----------------------------------------------------------------------
  -- Function streams
  -----------------------------------------------------------------------
  functions = {

    ---- Parameterless: flat {t, tp, v} arrays ----

    -- Zone text (scalar string)
    ["GetZoneText"] = {
      { t = 0.000, tp = 0.00015, v = "Dun Morogh" },
      { t = 90.000, tp = 90.00022, v = "Stormwind City" },
    },

    -- Loot item count (scalar number, 0 = no loot window)
    ["GetNumLootItems"] = {
      { t = 0.000, tp = 0.00018, v = 0 },
      { t = 45.200, tp = 45.20025, v = 2 },
      { t = 48.100, tp = 48.10012, v = 0 },
    },

    -- Quest log membership (synthetic: object, number[])
    ["QuestLog"] = {
      { t = 0.000, tp = 0.00019, v = { 12345 } },
      { t = 0.500, tp = 0.50025, v = { 12345, 56789 } },
      { t = 50.000, tp = 50.00010, v = { 12345 } },
    },

    -- Faction display order (synthetic: object, number[])
    ["FactionOrder"] = {
      { t = 0.000, tp = 0.00042, v = { 47, 72, 54, 69, 930, 509, 87, 21 } },
    },

    ---- Parameterized by "player" ----

    -- UnitLevel (scalar number)
    ["UnitLevel"] = {
      ["player"] = {
        { t = 0.000, tp = 0.00020, v = 12 },
        { t = 5.213, tp = 5.21350, v = 13 },
      },
    },

    -- UnitRace (tuple, n=3)
    ["UnitRace"] = {
      ["player"] = {
        { t = 0.000, tp = 0.00023, v = { "Dwarf", "Dwarf", 3, n = 3 } },
      },
    },

    -- Map position (object {x, y}, rounded to 4 decimals)
    ["C_Map.GetPlayerMapPosition"] = {
      ["player"] = {
        { t = 0.000, tp = 0.00022, v = { x = 0.5477, y = 0.5486 } },
        { t = 5.200, tp = 5.20010, v = { x = 0.5520, y = 0.5539 } },
        { t = 90.000, tp = 90.00027, v = { x = 0.6111, y = 0.7422 } },
      },
    },

    ---- Parameterized by questId ----

    -- IsQuestComplete (scalar boolean)
    ["IsQuestComplete"] = {
      [56789] = {
        { t = 0.500, tp = 0.50030, v = false },
        { t = 49.000, tp = 49.00015, v = true },
      },
    },

    -- Quest objectives (object, QuestObjectiveInfo[])
    ["C_QuestLog.GetQuestObjectives"] = {
      [56789] = {
        { t = 0.500, tp = 0.50032, v = {
          { text = "Tough Condor Meat: 0/8", type = "item", finished = false, numFulfilled = 0, numRequired = 8 },
        }},
        { t = 49.000, tp = 49.00020, v = {
          { text = "Tough Condor Meat: 8/8", type = "item", finished = true, numFulfilled = 8, numRequired = 8 },
        }},
      },
    },

    -- C_QuestLog.GetInfo (object, table): raw API return, only present when
    -- C_QuestLog.GetInfo exists on this client
    ["C_QuestLog.GetInfo"] = {
      [56789] = {
        { t = 0.500, tp = 0.50033, v = { title = "A New Threat", level = 2, isHeader = false, questID = 56789 } },
      },
    },

    -- C_QuestLog.IsComplete (scalar boolean/nil): raw API return, independent
    -- of C_QuestLog.GetInfo -- there is no synthesized composite value
    ["C_QuestLog.IsComplete"] = {
      [56789] = {
        { t = 0.500, tp = 0.50034, v = false },
        { t = 49.000, tp = 49.00022, v = true },
      },
    },

    -- Legacy GetQuestLogTitle (tuple, n=17): only present when this global
    -- exists; on such clients it is recorded as its own raw tuple, not
    -- derived from C_QuestLog.GetInfo/IsComplete above
    -- Returns: title, level, suggestedGroup, isHeader, isCollapsed, isComplete,
    --          frequency, questID, startEvent, displayQuestID, isOnMap,
    --          hasLocalPOI, isTask, isBounty, isStory, isHidden, isScaling
    ["GetQuestLogTitle"] = {
      [56789] = {
        { t = 0.500, tp = 0.50033, v = { "A New Threat", 2, 0, false, false, false, 0, 56789, false, false, false, false, false, false, false, false, false, n = 17 } },
        { t = 49.000, tp = 49.00021, v = { "A New Threat", 2, 0, false, false, true, 0, 56789, false, false, false, false, false, false, false, false, false, n = 17 } },
      },
    },

    -- QuestLogZone (scalar string/nil): derived header/zone title preceding
    -- this quest in the quest log, resolved while iterating quest-log rows
    ["QuestLogZone"] = {
      [56789] = {
        { t = 0.500, tp = 0.50033, v = "Westfall" },
      },
    },

    ---- Parameterized by slot index ----

    -- GetLootSlotInfo (tuple, n=9; nil when loot window closed)
    ["GetLootSlotInfo"] = {
      [1] = {
        { t = 45.200, tp = 45.20030, v = { "Icon\\Path", "Copper Coin", 1, n = 9 } },
        { t = 48.100, tp = 48.10015, v = nil },
      },
      [2] = {
        { t = 45.200, tp = 45.20031, v = { "Icon\\Path", "Linen Cloth", 2, n = 9 } },
        { t = 48.100, tp = 48.10016, v = nil },
      },
    },

    ---- Parameterized by factionID ----

    -- C_Reputation.GetFactionDataByID (object, table): recorded whenever this
    -- modern API exists. On clients without it, the legacy GetFactionInfoByID
    -- tuple below is recorded instead -- never both, never synthesized.
    ["C_Reputation.GetFactionDataByID"] = {
      [47] = {  -- Ironforge
        { t = 0.000, tp = 0.00043, v = { name = "Ironforge", factionID = 47, reaction = 5, currentStanding = 4500 } },
        { t = 120.500, tp = 120.50018, v = { name = "Ironforge", factionID = 47, reaction = 5, currentStanding = 4520 } },
      },
    },

    -- Legacy GetFactionInfoByID (tuple, n=16): only present when
    -- C_Reputation.GetFactionDataByID is unavailable
    ["GetFactionInfoByID"] = {
      [47] = {  -- Ironforge
        { t = 0.000, tp = 0.00043, v = {
          "Ironforge", "Home city of the Dwarves.", 5, 3000, 9000, 4500,
          false, false, false, false, true, false, 47, true, false, false,
          n = 16,
        }},
        { t = 120.500, tp = 120.50018, v = {
          "Ironforge", "Home city of the Dwarves.", 5, 3000, 9000, 4520,
          false, false, false, false, true, false, 47, true, false, false,
          n = 16,
        }},
      },
      [72] = {  -- Stormwind
        { t = 0.000, tp = 0.00044, v = {
          "Stormwind", "Alliance capital.", 6, 9000, 21000, 15200,
          false, false, false, false, true, false, 72, false, false, false,
          n = 16,
        }},
      },
    },
  },

  -----------------------------------------------------------------------
  -- Delta streams (functionsDelta)
  -----------------------------------------------------------------------
  functionsDelta = {
    -- C_QuestLog.GetAllCompletedQuestIDs: initial set + ordered delta entries.
    -- The legacy GetQuestsCompleted global, when it exists, would appear here
    -- as its own fully independent stream -- never merged with this one.
    ["C_QuestLog.GetAllCompletedQuestIDs"] = {
      t = 0,
      tp = 0,
      initial = { 123, 456, 789 },
      delta = {
        { t = 50.000, tp = 50.00012, add = { 56789 } },
      },
    },
  },
}
```
