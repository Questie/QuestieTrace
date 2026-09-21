# QuestieTrace Specs

Agent-facing index for QuestieTrace, a WoW Classic Era addon that records quest/gameplay events and WoW API return streams for offline replay. Schema version 9.

---

## Quick Start (Read Order)

1. [ARCHITECTURE_SPEC.md](./ARCHITECTURE_SPEC.md) - File structure, load order, bootstrap
2. [SCHEMA_SPEC.md](./SCHEMA_SPEC.md) - SavedVariables structure and time model
3. [TRACKER_SPEC.md](./TRACKER_SPEC.md) - Tracker architecture, lifecycle, trigger patterns
4. [EVENT_CATALOG.md](./EVENT_CATALOG.md) - Event registration by tracker
5. [FUNCTION_EMULATION_SPEC.md](./FUNCTION_EMULATION_SPEC.md) - Reconstruct WoW API calls from captured data
6. [UI_SPEC.md](./UI_SPEC.md) - Control frame, slash commands, button states
7. [LuaLS_annotations.md](./LuaLS_annotations.md) - Lua type annotation reference

---

## System Map

```text
┌─────────────────────────────────────────────────────────┐
│  globals.lua          Core namespace, utilities,        │
│                       RegisterTracker/RegisterDump APIs │
├─────────────────────────────────────────────────────────┤
│  Trackers/            13 isolated tracker files         │
│    PlayerIdentity     Race/class/base class/sex/faction │
│    UnitLevel          UnitLevel + quest green range     │
│    Position           Zone/map/XY/instance state        │
│    Loot               Loot window functions             │
│    Reputation         FactionOrder, faction detail raw │
│    QuestLog           Quest/reward/timer/text streams   │
│    CompletedQuests    GetQuestsCompleted delta stream   │
│    UnitInteraction    Unit identity + gossip quest APIs │
│    GroupState         Party membership state            │
│    SkillLines         Skill window + profession tabs    │
│    SpellBook          Raw slot state + known spell set  │
│    QuestDialog        Gossip/greeting/current quest UI  │
│    ResetTime          Server/reset-time snapshots       │
├─────────────────────────────────────────────────────────┤
│  Dumps/               Map hierarchy dump provider       │
├─────────────────────────────────────────────────────────┤
│  Export/              Encoding.lua: CBOR/Deflate codec   │
│                       Export.lua: scrub + serialize     │
│                       ExportUI.lua: export window only  │
├─────────────────────────────────────────────────────────┤
│  QuestieTrace_UI     Control frame + auto-start toggle  │
├─────────────────────────────────────────────────────────┤
│  QuestieTrace        Event bus, session lifecycle,      │
│                       slash commands, bootstrap         │
└─────────────────────────────────────────────────────────┘
```

Primary JTBDs:

- Capture quest progression and gameplay events in WoW Classic Era.
- Save detailed session data for offline analysis.
- Enable replay/emulation of WoW API function calls at any timestamp.
- Preserve low-churn dumps separately from per-character trace sessions.

---

## Spec Layout

```text
specs/
  README.md
  ARCHITECTURE_SPEC.md
  SCHEMA_SPEC.md
  TRACKER_SPEC.md
  EVENT_CATALOG.md
  FUNCTION_EMULATION_SPEC.md
  UI_SPEC.md
  TRACE_FILE_SPEC.md
  LuaLS_annotations.md
```

---

## Tracker Summary

| Tracker | Events | Trigger Pattern | Purpose |
|---|---|---|---|
| PlayerIdentity | None | Init only | `UnitRace`, `UnitClass`, `UnitClassBase`, `UnitSex`, `UnitFactionGroup` |
| UnitLevel | 3 events | Event-driven | `UnitLevel["player"]`, `GetQuestGreenRange` |
| Position | zone/login events + timer + private movement sampling | Timer + event-driven | Zone texts, map ID, XY, instance state |
| Loot | `LOOT_READY`, `LOOT_CLOSED` | Window lifecycle | Loot count + per-slot functions |
| Reputation | 5 events | Event + index iteration | `FactionOrder`, `GetFactionInfoByID`/`C_Reputation.GetFactionDataByID` |
| QuestLog | quest/login events | Event + delayed re-samples + iteration | Quest membership, per-quest text/data, timers, rewards |
| CompletedQuests | quest/login events | Event + delayed re-samples | `GetQuestsCompleted` delta stream |
| UnitInteraction | target/dialog/NPC/login events | Fixed unit-token fanout | `UnitGUID`, `UnitName`, gossip quest-list APIs |
| GroupState | 5 events | Event-driven | `IsInGroup`, `GetNumGroupMembers` |
| SkillLines | 3 events | Event + index iteration | `GetNumSkillLines`/`GetSkillLineInfo` (legacy, when present), `GetProfessions`, `GetProfessionInfo`, `C_TradeSkillUI.*` (independent, when present) |
| SpellBook | 2 events | Event + slot iteration | Spellbook slot streams + `PlayerKnownSpells` delta |
| QuestDialog | 8 dialog events | Event + delayed re-samples + observed close sample | Gossip/greeting/current quest dialog APIs |
| ResetTime | 3 lifecycle events | Init + snapshots | `GetServerTime`, `GetQuestResetTime`/`C_DateAndTime.GetSecondsUntilDailyReset` (whichever present) |

---

## Conventions

- Schema version is 9.
- All timestamps are session-relative.
- All Lua code must use LuaLS annotations.
- Function streams can be flat, one-level parameterized, or nested parameterized (`functions[name][arg1][arg2]...`) for true multi-argument APIs.
- Packed args use `{ ..., n = count }`.
- Missing `v` on a stream entry means a stored nil value.
- `GetQuestsCompleted` and `PlayerKnownSpells` use delta streams.
- Update specs when implementation changes.

---

## When You Need X, Read Y

| Need | Read |
|---|---|
| File structure and load order | [ARCHITECTURE_SPEC](./ARCHITECTURE_SPEC.md) |
| SavedVariables and stream shapes | [SCHEMA_SPEC](./SCHEMA_SPEC.md) |
| How trackers work | [TRACKER_SPEC](./TRACKER_SPEC.md) |
| What events trigger what | [EVENT_CATALOG](./EVENT_CATALOG.md) |
| Replay lookup and nested streams | [FUNCTION_EMULATION_SPEC](./FUNCTION_EMULATION_SPEC.md) |
| UI behavior and slash commands | [UI_SPEC](./UI_SPEC.md) |
| Trace file standalone guide | [TRACE_FILE_SPEC](./TRACE_FILE_SPEC.md) |
