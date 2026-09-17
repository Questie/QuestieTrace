# Architecture Spec (v9)

File structure, load order, bootstrap sequence, SavedVariables management, and event/dump routing for QuestieTrace.

---

## 1) File load order

Defined by `QuestieTrace-Classic.toc`:

```text
Libs/LibStub/LibStub.lua              -- Library stub for LibDeflate
Libs/LibDeflate/LibDeflate.lua        -- Compression library (CBOR + Deflate)
Modules/globals.lua                   -- Core namespace, utilities, RegisterTracker/RegisterDump APIs
Modules/Localization/l10n.lua         -- Localization system (Core.l10n)
Modules/Localization/Translations/ExportUI.lua -- Export/UI translations
Modules/Localization/Translations/ExportReminder.lua -- Share-reminder translations
Modules/Trackers/PlayerIdentity.lua   -- UnitRace, UnitClass, UnitClassBase, UnitSex, UnitFactionGroup (t=0 only)
Modules/Trackers/UnitLevel.lua        -- UnitLevel["player"], GetQuestGreenRange
Modules/Trackers/Position.lua         -- Zone texts, map ID, player position, instance state
Modules/Trackers/Loot.lua             -- Loot window capture
Modules/Trackers/Reputation.lua       -- Faction reputation
Modules/Trackers/QuestLog.lua         -- Quest log membership + per-quest/reward/timer functions
Modules/Trackers/CompletedQuests.lua  -- GetQuestsCompleted delta stream
Modules/Trackers/QuestDialog.lua      -- Gossip, greeting, and current quest-dialog APIs
Modules/Trackers/UnitInteraction.lua  -- UnitGUID/UnitName and gossip quest-list APIs
Modules/Trackers/GroupState.lua       -- IsInGroup, GetNumGroupMembers
Modules/Trackers/SkillLines.lua       -- Skill window + profession tabs
Modules/Trackers/SpellBook.lua        -- Raw spellbook slots + PlayerKnownSpells
Modules/Trackers/ResetTime.lua        -- GetServerTime and GetQuestResetTime
Modules/Dumps/MapHierarchy.lua        -- Static C_Map hierarchy dump (PLAYER_LOGIN + /qlt dumpmap)
Modules/Export/Encoding.lua           -- CBOR/Deflate encoding for export payloads
Modules/Export/Export.lua             -- Payload building, privacy scrubbing
Modules/Export/ExportUI.lua           -- Export window UI
Modules/Export/ExportReminder.lua     -- Share reminder: chat notification + export hyperlink
QuestieTrace_UI.lua                   -- Control frame UI
QuestieTrace.lua                      -- Entry point: session lifecycle, event bus, slash commands
```

### Load order rationale

1. `LibStub.lua` / `LibDeflate.lua` provide the compression codec used by the Export subsystem.
2. `globals.lua` establishes `QuestieTraceCore`, shared types, utility helpers, tracker registration, and dump registration.
3. `l10n.lua` + the `Translations/` files initialize the localization system (`Core.l10n`) before any module calls `l10n(...)`.
4. Tracker files call `Core.RegisterTracker` at file scope so event routing tables exist before the event frame is created.
5. Dump files call `Core.RegisterDump` at file scope so dump event/slash routing exists before bootstrap.
6. `Encoding.lua` provides CBOR/Deflate encoding functions used by `Export.lua`.
7. `Export.lua` builds and scrubs export payloads (depends on `Encoding.lua` and `Core.l10n`).
8. `ExportUI.lua` defines the export window (depends on `Core.l10n` and `Export.lua`).
9. `ExportReminder.lua` installs the chat hyperlink handler at file scope and defines
   `Core.StartShareReminders()`, which `QuestieTrace.lua` calls on `PLAYER_LOGIN`. It loads after
   `ExportUI.lua` because the hyperlink opens `Core.ShowExportWindow()`.
10. `QuestieTrace_UI.lua` defines optional UI functions used by the main file.
11. `QuestieTrace.lua` runs last, creates the event frame, registers tracked events, and handles slash commands.

---

## 2) Bootstrap sequence

On `VARIABLES_LOADED`:

1. `EnsureSavedVariables()` validates/initializes saved data and settings.
   - If `QuestieTraceCharacter.currentSession` exists (leftover from a `/reload` or logout without explicit Save/Reset), it is restored into the in-memory
     `capture.session`, forced into a stopped state (filling `stoppedAt`/`duration` if missing, since tracking cannot safely resume across reload), and
     `capture.active` is set to `false`. This makes `Core.GetCaptureState()` report `"stopped_unsaved"` automatically; the existing Save/Reset buttons in
     `QuestieTrace_UI.lua` work without changes.
2. `Core.BuildControlFrame()` creates the optional control frame.
3. `Core.UpdateControlFrameStatus()` sets initial UI state.

`VARIABLES_LOADED` is consumed by bootstrap and is not recorded as a normal trace event.

After bootstrap:

- `PLAYER_LOGIN` starts capture automatically when `QuestieTrace.settings.autoStart ~= false`; this happens before event processing so `PLAYER_LOGIN` is the first event in an auto-started session.
- `PLAYER_LOGOUT` is processed first, then an active capture is saved, so logout is included in the saved session.

---

## 3) Event registration

All events from `TRACKED_EVENT_CATEGORIES` are registered on the main event frame with `pcall`, allowing Classic-version differences to be skipped safely.

Current categories are organizational only and are not persisted:

- `quest_state`
- `quest_dialog`
- `npc_interaction`
- `initialization`
- `player_state`
- `map_zone`
- `chat_system`
- `group_world`
- `inventory`

Tracker routing is separate: `Core._trackerCallbacks[event]` controls which trackers sample for a specific event. `ADDON_LOADED` is filtered so only `QuestieTrace`'s own load event is recorded.

---

## 4) SavedVariables

### Account-level: `QuestieTrace`

```lua
QuestieTrace = {
  schemaVersion = 9,
  settings = {
    maxSessions = 20,
    autoStart = true,
  },
}
```

### Account-level: `QuestieTraceDumps`

```lua
QuestieTraceDumps = {
  schemaVersion = 1,
  dumps = {
    map_hierarchy = MapHierarchyDumpData,
  },
}
```

### Per-character: `QuestieTraceCharacter`

```lua
QuestieTraceCharacter = {
  lastSavedSession = "2026-02-10_12-34-56", -- set on save only
  currentSession = SessionRecord?,          -- live/stopped-unsaved session, linked by reference
  sessions = { SessionRecord, ... },
  savedSessionCounter = 0,                  -- monotonic count of sessions ever saved
  reminder = {
    sessionCounterAtExport = 0,             -- savedSessionCounter when the export window was last opened
  },
}
```

`currentSession` is a direct reference to the in-memory `capture.session` table established by `Core.StartCapture()`. Since trackers mutate the table in place,
no periodic sync is needed — the reference remains valid for the session's lifetime. It is cleared by `Core.SaveCapture()` (session moved to `sessions[]`) and
`Core.ResetCapture()` (session explicitly discarded). On `VARIABLES_LOADED`, if a leftover `currentSession` exists, it is recovered as a stopped-unsaved
session (see Bootstrap sequence).

Each `SessionRecord` (saved or live) may also carry `exportedAt`, set once it has actually been shown in the export window — see "Export dedup" in
`specs/SCHEMA_SPEC.md`. This is separate from `reminder.sessionCounterAtExport`, which only gates the chat reminder; `exportedAt` is what prevents the same
session's data from being bundled into the export payload twice.

### Migration behavior

- If `QuestieTrace` is missing or has a non-v9 schema, the account settings table is recreated with defaults.
- Missing/invalid `settings.maxSessions` resets to 20.
- Missing `settings.autoStart` defaults to `true`.
- `QuestieTraceDumps` and its `dumps` table are ensured.
- Existing `QuestieTraceCharacter.sessions` data is preserved when it is already a table; otherwise it is initialized to an empty table.
- Missing `QuestieTraceCharacter.savedSessionCounter` initializes to the current `#sessions`, so existing users start from their current save count.
- Missing `QuestieTraceCharacter.reminder.sessionCounterAtExport` initializes to `0`, so existing users with saved data are reminded on their next login.
- These per-character fields are shape-checked rather than schema-gated, so adding them required no `SCHEMA_VERSION` bump (a bump would reset every user's account settings).

---

## 5) Session pruning and naming

After every save, sessions over `QuestieTrace.settings.maxSessions` are pruned, preferring already-exported sessions (oldest-first among them) over
never-exported ones — see "Session pruning" in `specs/SCHEMA_SPEC.md`. If no name is supplied at start/save, sessions use `date("%Y-%m-%d_%H-%M-%S")`;
`/qlt save [name]` overrides a start-time name.

---

## 6) Auto-start and control surface

`QuestieTrace.settings.autoStart` controls login capture. It defaults to `true` and can be toggled by `/qlt auto` or the control-frame checkbox.

Slash command aliases are `/questietrace` and `/qlt`:

| Command | Purpose |
|---|---|
| `/qlt start [name]` | Start a capture session |
| `/qlt stop` | Stop the active capture session |
| `/qlt save [name]` | Save the current capture, auto-stopping first if needed |
| `/qlt reset` | Discard an unsaved stopped session |
| `/qlt status` | Print capture status |
| `/qlt auto` | Toggle auto-start on login |
| `/qlt export` | Show the export window (never-exported sessions only) |
| `/qlt export all` | Show the export window, forcing already-exported sessions back in (e.g. to resend after a failed submission) |
| `/qlt dumpmap` | Run the map hierarchy dump provider |
| `/qlt ui` | Toggle the control frame |

For bridge-based diagnostics and tests, `Core.GetDiagnosticSession()` returns
`(session, source)` where `source` is `"active"`, `"stopped_unsaved"`,
`"saved"`, or `"none"`. It prefers the live capture session, then a recovered
`currentSession` (which also reports `"stopped_unsaved"`), then the newest
saved session. The returned table is not copied and must be treated as read-only
by callers.

---

## 7) Tracker summary

| Tracker | Purpose |
|---|---|
| PlayerIdentity | Player race/class/base-class/sex/faction at capture start |
| UnitLevel | Level and green quest range |
| Position | Zone, map, XY, instance state |
| Loot | Loot window count and per-slot APIs |
| Reputation | Faction order and faction detail streams |
| QuestLog | Quest membership, per-quest data, text, timers, rewards |
| CompletedQuests | Completed quest set as a delta stream |
| UnitInteraction | Target/NPC/quest NPC identity and C_Gossip quest-list APIs |
| GroupState | Group membership/count |
| SkillLines | Skill rows and profession tabs |
| SpellBook | Raw spellbook slots and known spell set |
| QuestDialog | Gossip, greeting, and current quest dialog APIs |
| ResetTime | Server/reset-time snapshots |

---

## 8) Shared utilities (globals.lua)

| Function | Purpose | Used by |
|---|---|---|
| `Core.RegisterTracker(def)` | Register a tracker definition | All trackers |
| `Core.RegisterDump(def)` | Register dump providers | Dump files |
| `Core.RunDumpsForEvent(event, ...)` | Run event-triggered dumps | Main event handler |
| `Core.RunDumpBySlash(action, ...)` | Run slash-triggered dumps | Slash commands |
| `Core.GetDumpHelpLines()` | Contribute dump help text | `/qlt help` |
| `Core.PackArgs(...)` | Pack varargs into `{..., n=count}` | Events and tuple streams |
| `Core.CopyPacked(args)` | Copy packed args | Event recording |
| `Core.Round(num, decimals)` | Round numbers | Position tracker |
| `DeepCompare(t1, t2)` | Recursive table comparison | Change detection |
| `C_After` / `Defer` | Timer helpers | Delayed sampling |
