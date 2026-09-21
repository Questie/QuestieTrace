# Trace File Spec

A QuestieTrace trace file is a Lua table representing a recorded gameplay
session from World of Warcraft Classic Era. It captures WoW API function
return values and game events over time, enabling offline replay without a
running game client.

Given this spec and a trace file, an implementor can build a complete
function emulation layer that answers "what did `FunctionX(arg)` return at
time T?" for every captured function.

---

## 1) SessionRecord — top-level structure

```lua
SessionRecord = {
  schemaVersion = 9,
  recordingContractVersion = 1,
  name = "2026-02-10_12-34-56",       -- session identifier

  -- Absolute clock baselines (seconds)
  startedAt        = 100000.000,       -- GetTime() at capture start
  startedAtPrecise = 4821.31204,       -- GetTimePreciseSec() at capture start
  stoppedAt        = 100333.150,       -- GetTime() at capture stop
  stoppedAtPrecise = 5154.46238,       -- GetTimePreciseSec() at capture stop
  duration         = 333.150,          -- stoppedAt - startedAt
  durationPrecise  = 333.15034,        -- stoppedAtPrecise - startedAtPrecise

  events         = { EventEntry, ... },
  functions      = { [functionKey] = FunctionStream, ... },
  functionsDelta = { [functionKey] = DeltaStream, ... },
}
```

`recordingContractVersion = 1` identifies the observed-only raw API contract
used by new captures. Synthetic/derived streams remain exceptions as documented
below. An absent marker means legacy/unknown semantics, including for schema v9:
those sessions may contain invented resets or normalized API returns. Consumers
must recognize version `1` explicitly; unsupported versions are unknown. Do not
backfill this field on older sessions. Existing saves/settings are preserved and
the storage schema remains v9.

- `events` — ordered time-series of game events that fired during the session.
- `functions` — captured WoW API return values over time (change-only).
- `functionsDelta` — large-set functions stored as initial + deltas.

---

## 2) Time model

All `t` and `tp` values inside entries are **session-relative** — seconds
since capture start. Two clocks are stored on every entry:

| Field | Source | Behavior |
|---|---|---|
| `t` | `GetTime()` | Cached once per frame. Multiple samples in the same frame share the same `t`. |
| `tp` | `GetTimePreciseSec()` | Monotonic, millisecond precision, unique per call. |

To convert back to absolute time:

```lua
absolute_t  = session.startedAt        + entry.t
absolute_tp = session.startedAtPrecise + entry.tp
```

For replay purposes, use `t` as the primary timeline. `tp` exists for
sub-frame ordering when multiple entries share the same `t`.

---

## 3) Events

```lua
{ t = 0.500, tp = 0.50021, e = "QUEST_ACCEPTED", a = { 101, 56789, n = 2 } }
```

| Field | Type | Description |
|---|---|---|
| `t` | number | Session-relative GetTime() |
| `tp` | number | Session-relative GetTimePreciseSec() |
| `e` | string | Event name |
| `a` | PackedArgs | Event arguments (see section 6) |

Events are ordered by `t` (ascending). To replay events in a time range:

```lua
for _, event in ipairs(session.events) do
  if event.t >= t_start and event.t <= t_end then
    fireEvent(event.e, unpack(event.a, 1, event.a.n))
  end
end
```

---

## 4) Function streams

All captured WoW API return values live in `session.functions`. Two stream
formats exist:

### Parameterless — flat array

The function takes no relevant arguments. The stream is a flat array of
`{t, tp, v}` entries:

```lua
["GetZoneText"] = {
  { t = 0.000, tp = 0.00015, v = "Dun Morogh" },
  { t = 90.000, tp = 90.00022, v = "Stormwind City" },
}
```

### Parameterized — keyed by argument

The function takes an argument that determines which stream to use. The
top-level table is keyed by that argument, each value is either a flat
`{t, tp, v}` array or another parameter map for true multi-argument APIs:

```lua
["UnitLevel"] = {
  ["player"] = {
    { t = 0.000, tp = 0.00020, v = 12 },
    { t = 5.213, tp = 5.21350, v = 13 },
  },
}
```

Keys can be strings (`"player"`, `"target"`) or numbers (quest IDs, slot
indices, faction IDs). Multi-argument streams recurse in native argument order:

```lua
["GetQuestLogRewardInfo"] = {
  [1] = { -- rewardIndex
    [783] = { -- questId
      { t = 0.000, tp = 0.00020, v = { "Reward", 134400, 1, 1, true, 12345, 10, n = 7 } },
    },
  },
}
```

### How to detect the format

```lua
local stream = session.functions[functionKey]
local firstKey = next(stream)
if type(stream[firstKey]) == "table" and stream[firstKey].t then
  -- parameterless: stream itself is the array
else
  -- parameterized: stream[param] is an array or another parameter map
end
```

### Change-only storage

Entries are only appended when the return value changes. Core streams
sampled at session start often have an initial entry at `t = 0`, but this
is not guaranteed for every stream. Discovered parameter streams and
transient dialog/gossip/reward/loot/indexed streams may first appear later;
streams for unavailable client APIs may be empty or absent entirely.

**To find the value at a target time:** scan the array and take the last
entry whose `t <= target_t`. If no entry exists at or before `target_t`,
the function was not yet captured (return nil).

### Nil as a valid value

When a function returns nil (e.g., loot functions after the loot window
closes), that nil is stored as a change entry. Due to Lua serialization,
`{ t = ..., tp = ..., v = nil }` becomes `{ t = ..., tp = ... }` with
the `v` key absent. This is transparent: `entry.v` evaluates to nil
regardless. Treat nil entries as "the function returned nil at this time,"
not as "no data."

---

## 5) Delta streams

Functions that return large sets are stored in `session.functionsDelta`
as an initial snapshot plus ordered deltas. `C_QuestLog.GetAllCompletedQuestIDs`
and the legacy `GetQuestsCompleted` global are two fully independent delta
streams -- each only present when its own real API exists on the capturing
client, never merged into one value:

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

| Field | Type | Description |
|---|---|---|
| `initial` | number[] | Full set at capture start |
| `delta` | DeltaEntry[] | Ordered changes over time |
| `delta[i].add` | number[]? | IDs added (omitted if empty) |
| `delta[i].remove` | number[]? | IDs removed (omitted if empty) |

To reconstruct the set at a target time:

```lua
function getCompletedQuests(session, target_t)
  local data = session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
      or session.functionsDelta["GetQuestsCompleted"]
  local set = {}
  for _, id in ipairs(data.initial) do
    set[id] = true
  end
  for _, d in ipairs(data.delta) do
    if d.t > target_t then break end
    if d.add then
      for _, id in ipairs(d.add) do set[id] = true end
    end
    if d.remove then
      for _, id in ipairs(d.remove) do set[id] = nil end
    end
  end
  return set  -- lookup table: set[questID] == true means completed
end
```

---

## 6) PackedArgs encoding

Event arguments and tuple return values use packed-args tables:

```lua
{ value1, value2, value3, n = 3 }
```

The `n` field is the authoritative count. It must be used for unpacking
because nil values in the middle cause Lua to serialize sparse arrays
(indices are skipped). Always unpack with:

```lua
unpack(packed, 1, packed.n)
```

Never use `#packed` or `ipairs` — they stop at the first nil.

---

## 7) Return value format

The `v` field in a function stream entry is self-describing:

| `v` shape | Meaning | How to return it |
|---|---|---|
| `type(v) == "table" and v.n` | Packed tuple (multiple return values) | `return unpack(v, 1, v.n)` |
| `type(v) == "table"` (no `n` key) | Object/array (single table return) | `return v` |
| scalar (string, number, boolean) | Single return value | `return v` |
| nil (absent `v` key) | Function returned nil | `return nil` |

---

## 8) Core emulation algorithm

Three functions cover all stream lookups:

```lua
--- Get the stream array for a function, optionally narrowed by parameter.
---@param session SessionRecord
---@param name string        -- function key (e.g. "GetZoneText", "UnitLevel")
---@param ... any            -- argument keys for parameterized/nested streams
---@return FunctionStreamEntry[]?
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

--- Find the stored value at a target time (latest entry where t <= target_t).
---@param stream FunctionStreamEntry[]
---@param target_t number
---@return any
function valueAt(stream, target_t)
  local result = nil
  for i = 1, #stream do
    if stream[i].t > target_t then break end
    result = stream[i].v
  end
  return result
end

--- Unpack a stored value into proper return values.
---@param v any
---@return any ...
function emulate(v)
  if type(v) == "table" and v.n then
    return unpack(v, 1, v.n)
  end
  return v
end
```

Usage for any function:

```lua
local stream = getStream(session, "UnitLevel", "player")
local v = valueAt(stream, target_t)
return emulate(v)  -- returns: 13
```

This generic algorithm works for **every** function in the catalog. No
per-function special cases are needed (except derived functions and delta
streams, described below).

---

## 9) Complete function catalog

Every function key that can appear in `session.functions`, organized by
parameter type. In sessions marked `recordingContractVersion = 1`, raw API
function streams store observed successful API returns only. Event-driven
close/removal/count-shrink probes may record nil, zero, empty string, or empty
table values, but only when those values came from the API call
itself. Explicit synthetic/derived streams are marked below.

### Parameterless functions

Called with no arguments. Stream is a flat `{t, tp, v}` array.

| Function key | Return type | Description |
|---|---|---|
| `GetZoneText` | scalar string | Current zone name |
| `GetSubZoneText` | scalar string | Current subzone name |
| `GetRealZoneText` | scalar string | Real zone (ignores phasing) |
| `IsInInstance` | tuple (n=2) | inInstance, instanceType |
| `GetInstanceInfo` | tuple (n=10) | name, instanceType, difficultyID, ... |
| `GetNumLootItems` | scalar number | Number of loot slots (0 when no loot window) |
| `IsInGroup` | scalar boolean | Whether in a party/raid |
| `GetNumGroupMembers` | scalar number | Party/raid size (0 when solo) |
| `GetQuestGreenRange` | scalar number | XP green-range threshold |
| `C_GossipInfo.GetAvailableQuests` | object (table[]) | Array of available quests from an NPC |
| `C_GossipInfo.GetActiveQuests` | object (table[]) | Array of active quests at an NPC |
| `C_GossipInfo.GetNumAvailableQuests` | scalar number | Gossip available count |
| `C_GossipInfo.GetNumActiveQuests` | scalar number | Gossip active count |
| `C_GossipInfo.GetText` | scalar string/nil | Gossip text |
| `C_GossipInfo.GetOptions` | object table[] | Gossip options |
| `GetNumGossipAvailableQuests` | scalar number | Legacy gossip available count |
| `GetNumGossipActiveQuests` | scalar number | Legacy gossip active count |
| `GetGossipAvailableQuests` | tuple (n varies) | Legacy repeated 7-tuples |
| `GetGossipActiveQuests` | tuple (n varies) | Legacy repeated 6-tuples |
| `GetGreetingText` | scalar string/nil | Greeting text |
| `GetNumActiveQuests` | scalar number | Greeting active count |
| `GetNumAvailableQuests` | scalar number | Greeting available count |
| `GetQuestID` | scalar number | Current quest dialog ID |
| `GetTitleText` | scalar string/nil | Current quest title |
| `GetQuestText` | scalar string/nil | Current quest text |
| `GetObjectiveText` | scalar string/nil | Current objective text |
| `GetProgressText` | scalar string/nil | Current progress text |
| `GetRewardText` | scalar string/nil | Current reward text |
| `GetRewardXP` | scalar number/nil | Current reward XP |
| `IsQuestCompletable` | scalar boolean/nil | Current progress state |
| `GetNumQuestChoices` | scalar number | Current choice reward count |
| `C_QuestLog.GetMaxNumQuestsCanAccept` | scalar number | Quest log cap |
| `GetServerTime` | scalar number | Low-frequency snapshot |
| `GetQuestResetTime` | scalar number | Legacy global; only present when it exists |
| `C_DateAndTime.GetSecondsUntilDailyReset` | scalar number | Only present when this API exists |
| `GetNumSkillLines` | scalar number | Legacy global; only present when it exists, no synthesized fallback |
| `GetProfessions` | tuple (n=5) | Profession tab indices |
| `C_TradeSkillUI.GetAllProfessionTradeSkillLines` | object number[] | Tracked independently whenever the API exists |
| `QuestLog` | object number[] | Synthetic active quest IDs in quest-log order |
| `FactionOrder` | object number[] | Synthetic faction IDs in display order |
| `SpellBook` | object number[] | Synthetic ordered known spell IDs from slots |

### Parameterized by `"player"`

Called with `"player"` as the argument.

| Function key | Return type | Description |
|---|---|---|
| `UnitLevel` | scalar number | Player level |
| `UnitRace` | tuple (n=3) | localizedName, englishName, raceID |
| `UnitClass` | tuple (n=3) | localizedName, englishName, classID |
| `UnitClassBase` | tuple (n=2) | classFilename, classID |
| `UnitSex` | scalar number | Sex ID |
| `UnitFactionGroup` | tuple (n=2) | englishFaction, localizedFaction |
| `C_Map.GetBestMapForUnit` | scalar number | Map ID |
| `C_Map.GetPlayerMapPosition` | object {x, y}/nil | Derived compatibility: current-map coordinates rounded to 4 decimals; nil when map/position is unavailable, including when no position API call was possible |

### Parameterized by quest ID (number)

Called with a quest ID as the argument.

| Function key | Return type | Description |
|---|---|---|
| `IsQuestComplete` | scalar boolean | Whether quest is completable |
| `HaveQuestData` | scalar boolean/nil | Quest cache availability |
| `C_QuestLog.IsOnQuest` | scalar boolean/nil | Raw API result; probed again after quest leaves log |
| `C_QuestLog.IsQuestFlaggedCompleted` | scalar boolean/nil | Raw API result; related to but not derived from `GetQuestsCompleted` |
| `C_QuestLog.GetQuestObjectives` | object (table[]/nil) | Raw API result; probed again after quest leaves log |
| `C_QuestLog.GetInfo` | object (table) | Raw API result, keyed by questID after resolving current quest log index |
| `C_QuestLog.GetQuestTagInfo` | object (table/nil) | Raw API result |
| `C_QuestLog.IsComplete` | scalar boolean/nil | Raw API result |
| `C_QuestLog.IsFailed` | scalar boolean/nil | Raw API result |
| `C_QuestLog.GetLogIndexForQuestID` | scalar number/nil | Raw API result |
| `GetQuestLogTitle` | tuple (n=17) | Legacy global; raw tuple, only present when this global exists -- not synthesized from `C_QuestLog.*` |
| `GetQuestLogIndexByID` | scalar number/nil | Legacy global; raw API result |
| `GetQuestLogQuestText` | tuple (n=2) | questDescription, questObjectives |
| `QuestLogZone` | scalar string/nil | Derived: header/zone title immediately preceding this quest in the quest log |
| `GetQuestTimers` | scalar number/nil | Derived questId-keyed seconds-left |
| `GetQuestLogTimeLeft` | scalar number/nil | Derived questId-keyed seconds-left |
| `GetNumQuestLogRewards` | scalar number/nil | Reward count |
| `GetQuestLogRewardMoney` | scalar number/nil | Reward money |
| `GetQuestTagInfo` | tuple (n varies) | Tag info |

### Nested parameterized by native arguments

| Function key | Shape | Return type | Description |
|---|---|---|---|
| `GetQuestLogRewardInfo` | `[rewardIndex][questId]` | tuple (n=7) or nil | Raw reward item API result; previously observed indices are probed again after counts shrink or quest leaves log |

### Parameterized by greeting index

| Function key | Return type | Description |
|---|---|---|
| `GetActiveTitle` | tuple (n=2) | title, isComplete |
| `GetAvailableTitle` | scalar string/nil | available quest title |

### Parameterized by unit token (string: `"target"`, `"npc"`, `"questnpc"`)

| Function key | Return type | Description |
|---|---|---|
| `UnitGUID` | scalar string or nil | GUID; nil when no unit |
| `UnitName` | packed tuple (n varies) | observed `UnitName(token)` returns; no synthetic `UnitExists` mapping |

### Parameterized by loot slot index (number)

| Function key | Return type | Description |
|---|---|---|
| `GetLootSlotInfo` | tuple (n varies) | observed loot slot info return |
| `GetLootSourceInfo` | tuple (n varies) | observed loot source return |
| `GetLootSlotLink` | scalar string/nil | observed loot link return |
| `GetLootSlotType` | scalar number/nil | observed loot type return |
| `GetSpellBookItemName` | tuple (n varies) | observed spellbook name return |
| `GetSpellBookItemInfo` | tuple (n varies) | observed spellbook info return |
| `IsPassiveSpell` | scalar number/nil | observed passive marker return |

### Parameterized by skill/profession index / trade-skill line ID (number)

| Function key | Return type | Description |
|---|---|---|
| `GetSkillLineInfo` | tuple (n=13) | Legacy global; only present when `GetNumSkillLines` exists, no synthesized fallback |
| `GetProfessionInfo` | tuple (n=10) | Profession tab information |
| `C_TradeSkillUI.GetTradeSkillLineInfoByID` | object (table) | Raw API result, keyed by skill-line ID; tracked independently whenever the API exists |

### Parameterized by faction ID (number)

| Function key | Return type | Description |
|---|---|---|
| `GetFactionInfoByID` | tuple (n=16) | Legacy global; raw tuple, only present when the legacy global exists (independent of whether C_Reputation.GetFactionDataByID also exists) |
| `C_Reputation.GetFactionDataByID` | object (table) | Raw API result |

### Delta streams (in `functionsDelta`)

| Function key | Description |
|---|---|
| `C_QuestLog.GetAllCompletedQuestIDs` | Set of completed quest IDs (see section 5) |
| `GetQuestsCompleted` | Legacy global; independent set of completed quest IDs, tracked whenever this global exists (see section 5) |
| `PlayerKnownSpells` | Set of known spell IDs discovered from spellbook slots |

---

## 10) Synthetic functions

Three function keys are **not** direct WoW API names. They are computed by
the capture system but stored identically to other streams:

| Function key | Value type | Description |
|---|---|---|
| `QuestLog` | object (number[]) | Array of quest IDs currently in the player's quest log. Computed internally during capture from whichever of `C_QuestLog.GetInfo` or the legacy `GetQuestLogTitle` global exists (this internal lookup is not itself recorded to the trace). |
| `FactionOrder` | object (number[]) | Ordered array of faction IDs as displayed in the reputation panel. Computed internally during capture from whichever of `C_Reputation.GetFactionDataByIndex`/`GetNumFactions` or the legacy `GetFactionInfo`/`GetNumFactions` globals exist, expanding headers as needed (not itself recorded to the trace). |
| `SpellBook` | object (number[]) | Ordered unique spell IDs discovered by enumerating spellbook slots. |

These are read like any parameterless function:

```lua
local questIDs = valueAt(getStream(session, "QuestLog"), target_t)
-- questIDs = { 12345, 56789 }
```

---

## 11) Derived functions

Some WoW API functions can be reconstructed by combining stored streams
rather than being stored directly:

### `GetFactionInfo(index)` → from `FactionOrder` + `GetFactionInfoByID`/`C_Reputation.GetFactionDataByID`

```lua
function emulateFactionInfo(session, index, target_t)
  local order = valueAt(getStream(session, "FactionOrder"), target_t)
  if not order or not order[index] then return nil end
  local factionID = order[index]
  -- Only one of these two streams is present on a given client -- whichever
  -- real API it exposed at capture time.
  local stream = getStream(session, "C_Reputation.GetFactionDataByID", factionID)
      or getStream(session, "GetFactionInfoByID", factionID)
  if not stream then return nil end
  return emulate(valueAt(stream, target_t))
end
```

### `GetNumFactions()` → from `FactionOrder`

```lua
function emulateNumFactions(session, target_t)
  local order = valueAt(getStream(session, "FactionOrder"), target_t)
  return order and #order or 0
end
```

---

## 12) Events captured

The trace records all game events that fired during the session. These are
not function calls — they are signals from the game engine. Each event
entry has the name (`e`) and its arguments (`a`).

Events are useful for:
- Knowing **when** things happened (quest accepted, level up, loot opened)
- Driving replay logic (advance state when events fire)
- Correlating function stream changes with game actions

The full set of event names that can appear varies by session. Common
categories include:

- **Quest state:** `QUEST_ACCEPTED`, `QUEST_REMOVED`, `QUEST_TURNED_IN`, `QUEST_LOG_UPDATE`, ...
- **Quest dialog:** `QUEST_DETAIL`, `QUEST_PROGRESS`, `QUEST_COMPLETE`, `GOSSIP_SHOW`, ...
- **Player state:** `PLAYER_LEVEL_UP`, `PLAYER_LOGIN`, `PLAYER_ENTERING_WORLD`, ...
- **Loot:** `LOOT_READY`, `LOOT_CLOSED`
- **Zone/map:** `ZONE_CHANGED`, `ZONE_CHANGED_NEW_AREA`, ...
- **Chat/system:** `CHAT_MSG_LOOT`, `CHAT_MSG_COMBAT_XP_GAIN`, ...
- **Group:** `GROUP_JOINED`, `GROUP_LEFT`, `GROUP_ROSTER_UPDATE`
- **NPC interaction:** `MERCHANT_SHOW`, `TRAINER_SHOW`, `BANKFRAME_OPENED`, ...

---

## 13) Lua serialization quirks

When loading a trace file from Lua serialization (e.g., `dofile`,
`loadfile`, or WoW SavedVariables), be aware of these behaviors:

### Nil in packed args creates sparse arrays

When a function returns `(value1, nil, value3)`, the serialized form
skips the nil index:

```lua
-- Original:   { "hello", nil, "world", n = 3 }
-- Serialized: { [1] = "hello", [3] = "world", n = 3 }
```

Always iterate using `for i = 1, v.n` and accept that `v[i]` may be nil.
Never use `#v` or `ipairs(v)`.

### Missing `v` key means nil

An entry `{ t = 48.1, tp = 48.10015 }` with no `v` field means the
function returned nil at that time. Accessing `entry.v` returns nil,
which is the correct value.

### Table keys can be numbers or strings

Parameterized streams use whatever type the original argument was:
- `functions["UnitLevel"]["player"]` — string key
- `functions["GetLootSlotInfo"][1]` — number key
- `functions["IsQuestComplete"][56789]` — number key

---

## 14) Implementing a full replay emulator

To build a complete emulator from this spec:

### Step 1: Load the trace

```lua
-- From a standalone file:
local session = dofile("trace.lua")

-- Or from SavedVariables:
local sv = dofile("QuestieTrace.lua")  -- per-character file
local session = sv.sessions[1]          -- pick a session
```

### Step 2: Implement the core lookup

Copy the three functions from section 8 (`getStream`, `valueAt`,
`emulate`). These are the foundation.

### Step 3: Create WoW API shims

For each function in the catalog (section 9), create a shim that wraps
the core lookup:

```lua
-- Parameterless example:
function GetZoneText()
  return emulate(valueAt(getStream(session, "GetZoneText"), currentTime))
end

-- Parameterized example:
function UnitLevel(unit)
  return emulate(valueAt(getStream(session, "UnitLevel", unit), currentTime))
end

-- Delta stream example (checks both independent completed-quest streams):
function GetQuestsCompleted()
  return getCompletedQuests(session, currentTime)  -- section 5
end
```

### Step 4: Handle derived functions

Implement `GetFactionInfo` and `GetNumFactions` using the derivation
logic from section 11.

### Step 5: Advance time

Control `currentTime` to scrub through the session. The valid range is
`0` to `session.duration`. All function shims use this value to look up
the correct state.

### Step 6: Replay events (optional)

Iterate `session.events` and fire callbacks at the appropriate times
(section 3). This enables event-driven replay alongside function
emulation.

---

## 15) File origin

Trace files are produced by the QuestieTrace addon for World of Warcraft
Classic Era. They are stored in WoW's SavedVariables system:

- **Per-character:** `WTF/Account/<ACCOUNT>/SavedVariables/QuestieTrace.lua`
  contains `QuestieTraceCharacter` with a `sessions` array.
- **Account-level:** Same path at account scope, contains settings only.

Each session is a self-contained `SessionRecord`. It can be extracted and
used independently — no other data or game client needed.
