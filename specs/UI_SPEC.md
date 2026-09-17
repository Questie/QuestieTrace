# UI Spec (v9)

Slash command interface for QuestieTrace. No control frame or UI buttons.

---

## 1) Capture states

| `capture.active` | `capture.session` | State |
|---|---|---|
| `false` | `nil` | Idle |
| `true` | table | Running |

`Core.GetCaptureState()` returns `"idle"` or `"running"`. There is no longer a `"stopped_unsaved"` state — recovered sessions from a previous load are auto-finalized into `sessions[]` during `EnsureSavedVariables()`, so the addon never exposes an unsaved session to the player.

---

## 2) Slash commands

Aliases: `/questietrace` and `/qlt`.

| Command | Action |
|---|---|
| `/qlt` or `/qlt help` | Print help text |
| `/qlt status` | Print tracking status and current event count to chat |
| `/qlt tracking` | Toggle data collection on/off, effective immediately |
| `/qlt debug` | Toggle debug prints |
| `/qlt export` | Show the export window (see section 4) |
| `/qlt export all` | Force re-show export window, including already-exported sessions |
| `/qlt dumpmap` | Run map hierarchy dump provider, when registered |

### Behaviors

- `/qlt tracking` toggles `QuestieTrace.settings.autoStart` and prints `Tracking: enabled` or `disabled`. The change takes effect immediately: toggling off stops and saves any running capture; toggling on starts a new one.
- `/qlt status` prints human-readable status without stopping or modifying anything.
- `/qlt debug` and `/qlt export` produce immediate feedback.

---

## 3) Tracking toggle behavior

`QuestieTrace.settings.autoStart` controls whether the addon automatically starts capturing at login.

- **Default**: `true` (automatic capturing enabled).
- **When ON**: Capture starts automatically at `PLAYER_LOGIN` (and immediately if toggled on via `/qlt tracking` while logged in).
- **When OFF**: No capture runs. Toggling off mid-capture immediately finalizes and saves what has been collected so far, so no data is lost.

This replaces the old "start/stop/save" manual workflow. Tracking is now a single toggle: always either on (capturing) or off (not capturing). There is no explicit "save" action from the player's perspective.

---

## 4) Export window

`Export/Export.lua` and `Export/ExportUI.lua` are strictly separate:

- **`Export/Export.lua`** — data only. `Core.BuildExportPayload()` returns a deep-copied, privacy-scrubbed table of all saved sessions for the current character (`{ exportVersion, generatedAt, sessions }`). `Core.BuildExportString()` encodes it via CBOR + Deflate compression + print-safe encoding (LibDeflate:EncodeForPrint), producing a compact binary string suitable for copy-paste sharing. The string is prefixed with a plaintext version marker (`!QuestieTrace:<n>!`) so the export format/version is visible without decoding the payload. Scrubbing removes the `player` token from the `UnitName` and `UnitGUID` function streams so the player's own name/realm never leaves the client; NPC identity data is unaffected.

- **`Export/ExportUI.lua`** — UI only. `Core.ShowExportWindow()` lazily builds a movable frame with a multiline, scrollable, read-only-by-convention edit box. On show, it calls `Core.BuildExportString()` and populates the edit box, focuses it, and highlights all text so the user can immediately `Ctrl+A` / `Ctrl+C`. It never touches SavedVariables or session data itself. After marking sessions exported via `Core.MarkSessionsExported()`, it calls `Core.FinalizeLiveSessionIfExported()` to immediately save any live session that was just exported, preventing accidental re-bundling of later events into the same payload.

- **Deduplication**: Once a session is included in an export payload and shown to the user, it is marked with an `exportedAt` timestamp. `Core.BuildExportPayload()` skips any session with `exportedAt` already set (unless `/qlt export all` is used to force re-inclusion). This prevents duplicate data in submissions.

- **Finalization**: If the live (running) session was included in the export, `Core.FinalizeLiveSessionIfExported()` immediately saves it to `sessions[]` and (if tracking is enabled) starts a fresh capture, so new events go into a new, distinct session with its own export opportunity.

- Triggered by `/qlt export` or by the share-reminder chat link (section 5).

---

## 5) Share reminder

`Export/ExportReminder.lua` prints a chat notification reminding the player to share their captured data, with a clickable link that opens the export window.

### Message

```text
QuestieTrace: It is time to share your trace data. [Click here to open the export window]
```

The bracketed text is a `|Hquestietrace:export|h` hyperlink, printed via `DEFAULT_CHAT_FRAME:AddMessage`. Both strings are localized into all 10 supported locales (`Localization/Translations/ExportReminder.lua`).

### Triggers

| Trigger | Timing |
|---|---|
| Login | `PLAYER_LOGIN` schedules the first check 10 seconds later, so the message is not buried in login spam |
| Repeat | Every 1800 seconds (30 minutes) thereafter, for as long as the client stays loaded |

`Core.StartShareReminders()` is called from the `PLAYER_LOGIN` branch of the main event handler and is idempotent within one load. The reminder deliberately does **not** use `Core.RegisterTracker`: tracker callbacks only fire while a capture is active, which would silence the reminder exactly when the player has stopped capturing.

### Eligibility

`Core.IsShareDue()` returns true when there is unshared data. This includes:

1. Any saved session in `QuestieTraceCharacter.sessions` that lacks an `exportedAt` timestamp.
2. A live (running) session with at least one event and no `exportedAt` timestamp.

This is a cheap metadata check (no full payload build). Both cases are exportable, and both should prompt the player to share.

> **Consequence:** with default settings a capture auto-starts at login and auto-saves only on `PLAYER_LOGOUT`, so a brand-new character has zero saved sessions for their entire first play session and sees no reminder until their second login. A long-running unsaved live session also triggers a reminder on its own (no minimum-age threshold needed).

### Hyperlink handling

Clicks on `questietrace:export` route through `SetItemRef`. The handler is installed at load time, preferring `LinkUtil.RegisterLinkHandler` (which consumes the click, so `ItemRefTooltip` never opens) and falling back to `hooksecurefunc("SetItemRef", ...)` on clients without `LinkUtil` — there the stock handler has already opened an empty tooltip for the unknown link type, which the handler hides again. `SetItemRef` is never replaced, which would risk taint.

There is no opt-out setting or slash command.

---

## 6) StatusData (read-only diagnostic)

Returned by `Core.GetStatusData()`:

```lua
{
  captureState = "idle" | "running",
  isRunning    = boolean,
  sessionName  = string,
  eventCount   = number,
  canSave      = boolean,  -- deprecated; kept for compat but always false now
}
```

Used only by `/qlt status` to print a human-readable status line. No UI elements consume this.
