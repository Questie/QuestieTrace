# UI Spec (v9)

Control frame and slash command interface for QuestieTrace.

---

## 1) Capture states

| `capture.active` | `capture.session` | State | Label |
|---|---|---|---|
| `false` | `nil` | Idle | "Idle" |
| `true` | table | Running | "Running" |
| `false` | table | Stopped, unsaved | "Stopped (unsaved)" |

`Core.GetCaptureState()` returns `"idle"`, `"running"`, or `"stopped_unsaved"`.

---

## 2) Control frame

A movable frame at top center of the screen (`250x120`), created by `Core.BuildControlFrame()` on `VARIABLES_LOADED`.

### Elements

- **Title**: "QuestieTrace"
- **Start/Reset button**: 72x22, top-left
- **Stop button**: 72x22, right of Start
- **Save button**: 72x22, right of Stop
- **Status text**: below buttons, left-aligned, 230px wide
- **Auto-start checkbox**: bottom-left; checked when `QuestieTrace.settings.autoStart ~= false`; writes its checked state to `QuestieTrace.settings.autoStart`
- **Auto-start label**: "Auto-start on login"

### Status text

`Core.UpdateControlFrameStatus()` formats status as three lines:

```text
Status: <Idle|Running|Stopped (unsaved)>
Session: <sessionName>
Events: <eventCount>
```

### Button state table

| State | Start button | Stop button | Save button |
|---|---|---|---|
| Idle | **Start** (enabled) | disabled | disabled |
| Running | Start (disabled) | **Stop** (enabled) | disabled |
| Stopped, unsaved | **Reset** (enabled) | disabled | **Save** (enabled) |

In `stopped_unsaved`, clicking **Reset** calls `Core.ResetCapture()` and discards the unsaved session.

### OnUpdate polling

The control frame polls `Core.UpdateControlFrameStatus()` every 0.5 seconds via `OnUpdate`.

### Frame properties

- Clamped to screen
- Movable via left-button drag
- Dark background with border texture and darker inner fill
- Toggle visibility with `/qlt ui`

---

## 3) Slash commands

Aliases: `/questietrace` and `/qlt`.

| Command | Action |
|---|---|
| `/qlt` or `/qlt help` | Print help text |
| `/qlt start [name]` | Start capture with optional session name |
| `/qlt stop` | Stop active capture |
| `/qlt save [name]` | Save session, auto-stopping if running |
| `/qlt reset` | Discard unsaved capture when not running |
| `/qlt status` | Print status to chat |
| `/qlt auto` | Toggle auto-start on login |
| `/qlt dumpmap` | Run map hierarchy dump provider, when registered |
| `/qlt export` | Show the export window (see section 6) |
| `/qlt ui` | Toggle control frame visibility |

### Behaviors

- `/qlt save [name]` overrides the session name set at start time.
- `/qlt reset` prints an error and does nothing while capture is running; otherwise it clears any unsaved session and prints `Session discarded.`
- `/qlt auto` toggles `QuestieTrace.settings.autoStart` and prints `Auto-start on login: enabled` or `disabled`.
- If no name is provided, sessions are named with `YYYY-MM-DD_HH-MM-SS`.

---

## 4) Auto-start behavior

`QuestieTrace.settings.autoStart` defaults to `true` on fresh settings and when missing. When `PLAYER_LOGIN` fires and no capture is active, the main event handler starts capture automatically unless `autoStart == false`. Auto-start happens before event processing, so `PLAYER_LOGIN` is recorded as the first event in the new session.

The checkbox and `/qlt auto` update the same setting.

---

## 5) StatusData

```lua
{
  captureState = "idle" | "running" | "stopped_unsaved",
  isRunning    = boolean,
  sessionName  = string,
  eventCount   = number,
  canSave      = boolean,
}
```

---

## 6) Export window

`Export/Export.lua` and `Export/ExportUI.lua` are strictly separate:

- **`Export/Export.lua`** — data only. `Core.BuildExportPayload()` returns a
  deep-copied, privacy-scrubbed table of all saved sessions for the current
  character (`{ exportVersion, generatedAt, sessions }`). `Core.BuildExportString()`
  encodes it via CBOR + Deflate compression + print-safe encoding (LibDeflate:EncodeForPrint),
  producing a compact binary string suitable for copy-paste sharing. The
  string is prefixed with a plaintext version marker (`!QuestieTrace:<n>!`)
  so the export format/version is visible without decoding the payload.
  Scrubbing removes the `player` token from the `UnitName` and `UnitGUID` 
  function streams so the player's own name/realm never leaves the client; 
  NPC identity data is unaffected.
- **`Export/ExportUI.lua`** — UI only. `Core.ShowExportWindow()` lazily builds
  a movable frame with a multiline, scrollable, read-only-by-convention edit
  box. On show, it calls `Core.BuildExportString()` and populates the edit
  box, focuses it, and highlights all text so the user can immediately
  `Ctrl+A` / `Ctrl+C`. It never touches SavedVariables or session data itself.
  On show it calls `Core.MarkExportOpened()` (guarded) so the share reminder can
  record the visit; the SavedVariables write lives in `ExportReminder.lua`.
- Triggered by `/qlt export` or by the share-reminder chat link (section 7).

---

## 7) Share reminder

`Export/ExportReminder.lua` prints a chat notification reminding the player to
share their captured data, with a clickable link that opens the export window.

### Message

```text
QuestieTrace: It is time to share your trace data. [Click here to open the export window]
```

The bracketed text is a `|Hquestietrace:export|h` hyperlink, printed via
`DEFAULT_CHAT_FRAME:AddMessage`. Both strings are localized into all 10 supported
locales (`Localization/Translations/ExportReminder.lua`).

### Triggers

| Trigger | Timing |
|---|---|
| Login | `PLAYER_LOGIN` schedules the first check 10 seconds later, so the message is not buried in login spam |
| Repeat | Every 1800 seconds (30 minutes) thereafter, for as long as the client stays loaded |

`Core.StartShareReminders()` is called from the `PLAYER_LOGIN` branch of the main
event handler and is idempotent within one load. The reminder deliberately does
**not** use `Core.RegisterTracker`: tracker callbacks only fire while a capture is
active, which would silence the reminder exactly when the player has stopped capturing.

### Eligibility

`Core.IsShareDue()` returns true only when both hold:

1. `#QuestieTraceCharacter.sessions > 0` — only **saved** sessions are exportable
   (`Core.BuildExportPayload()` never includes the live session), so a player with
   nothing shareable is never prompted.
2. `savedSessionCounter > reminder.sessionCounterAtExport` — at least one session
   has been saved since the export window was last opened.

Each 30-minute tick re-checks eligibility and reschedules regardless of the result,
so data that becomes shareable mid-session still triggers a reminder.

> **Consequence:** with default settings a capture auto-starts at login and auto-saves
> only on `PLAYER_LOGOUT`, so a brand-new character has zero saved sessions for their
> entire first play session and sees no reminder until their second login.

### State

Per-character, on `QuestieTraceCharacter` (shape-checked, not schema-gated, so no
`SCHEMA_VERSION` bump is required):

| Field | Meaning |
|---|---|
| `savedSessionCounter` | Monotonic count of sessions ever saved; incremented in `Core.SaveCapture()` |
| `reminder.sessionCounterAtExport` | Value of `savedSessionCounter` when the export window was last opened |

`savedSessionCounter` is monotonic on purpose. `#sessions` is capped by
`PruneSessionsIfNeeded()`, so using it as the watermark would permanently suppress
reminders for any character sitting at `maxSessions`.

### Hyperlink handling

Clicks on `questietrace:export` route through `SetItemRef`. The handler is installed
at load time, preferring `LinkUtil.RegisterLinkHandler` (which consumes the click, so
`ItemRefTooltip` never opens) and falling back to `hooksecurefunc("SetItemRef", ...)`
on clients without `LinkUtil` — there the stock handler has already opened an empty
tooltip for the unknown link type, which the handler hides again. `SetItemRef` is
never replaced, which would risk taint.

There is no opt-out setting or slash command.
