# UI Spec (v10)

Slash command interface for QuestieTrace, with separate consent and export dialogs.

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
| `/qlt consent` | Show the data collection consent dialog |
| `/qlt export` | Show the export window (see section 4) |
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

- **`Export/Export.lua`** — data only. `Core.BuildExportPayload()` returns a deep-copied, privacy-scrubbed table of every saved session for the current character plus the live session if it has events (`{ exportVersion, generatedAt, sessions }`). There is no "already exported" state to filter — reported sessions are deleted outright rather than flagged, so every saved session is by definition unreported. `Core.BuildExportString()` encodes it via CBOR + Deflate compression + print-safe encoding (LibDeflate:EncodeForPrint), producing a compact binary string suitable for copy-paste sharing. The string is prefixed with a plaintext version marker (`!QuestieTrace:<n>!`) so the export format/version is visible without decoding the payload. Scrubbing removes the `player` token from the `UnitName` and `UnitGUID` function streams so the player's own name/realm never leaves the client; NPC identity data is unaffected.

- **`Export/ExportUI.lua`** — UI only. `Core.ShowExportWindow()` lazily builds a movable frame with a multiline, scrollable, read-only-by-convention edit box. On show, it calls `Core.BuildExportString()` and populates the edit box, focuses it, and highlights all text so the user can immediately `Ctrl+A` / `Ctrl+C`. It never touches SavedVariables or session data itself.

- **Explicit confirmation, not auto-dismissal**: Opening or closing the window never deletes anything. Two buttons make the outcome explicit: **Close** just hides the window (data remains pending and will be offered again next time), and **"I reported this"** calls `Core.ConfirmExportReported()`, which calls `Core.DeleteReportedSessions()` on the sessions shown — removing saved ones from `QuestieTraceCharacter.sessions` and, if the live session was included, discarding it (without saving) and starting a fresh capture — then hides the window. Both buttons carry a tooltip clarifying this distinction. "I reported this" is hidden whenever there is nothing valid to confirm (no exportable data, or the codec is unavailable so the shown text is only an error message) — the source sessions for the currently displayed string are tracked in a module-local `pendingSourceSessions`, cleared once confirmed.

- **Deletion, not flagging**: A session is only ever removed once the player explicitly confirms via "I reported this". Since reported sessions are deleted rather than marked, `Core.BuildExportPayload()` needs no dedup filtering — everything it returns is guaranteed unreported. This keeps SavedVariables small and ensures data is never lost just because the window was dismissed without confirming.

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

`Core.IsShareDue()` returns true when there is unreported data. This includes:

1. Any saved session in `QuestieTraceCharacter.sessions` — every entry there is guaranteed unreported, since reported sessions are deleted outright rather than flagged.
2. A live (running) session with at least one event that started at least 900 seconds (15 minutes) ago (`LIVE_SESSION_MIN_AGE`).

This is a cheap metadata check (no full payload build). Both cases are exportable, and both should prompt the player to share.

> **Consequence:** with default settings a capture auto-starts at login and auto-saves only on `PLAYER_LOGOUT`, so a brand-new character has zero saved sessions for their entire first play session. A live session triggers a reminder only once it is at least 15 minutes old: auto-start records `PLAYER_LOGIN`/`PLAYER_ENTERING_WORLD` immediately, so without the age threshold every login (including a brand-new character) would be reminded at the 10-second check. Thus the first-play-session reminder occurs at the 30-minute check.

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

---

## 7) Consent dialog

`Widgets/Dialog.lua` builds anonymous frames entirely in Lua and stores its interface on WoW's per-addon private table as `addon.Dialog`. Each addon owns its definitions, callbacks, frame construction, and sizing. There are no widget globals, mixins, or XML templates shared between Questie and QuestieTrace. Only positioning is shared through `LibPopupStack-1.0`, a LibStub library whose compatible upgrades retain registered frames and the single positioning driver.

`Modules/Consent.lua` registers `QUESTIETRACE_DATA_COLLECTION_CONSENT` after localization and the widget files load. There is no dedicated consent XML frame. `Core.ShowConsentPrompt()` returns the existing visible decision or shows the registered definition. Repeated requests preserve the pending frame and its data.

The alert layout starts at 420 units wide with a 290-unit centered text column, a 36-unit warning icon, and buttons at least 120-by-21 units with a 10-unit gap. Wrapped body text and button labels determine the final dimensions. Layout is checked when shown, on the next frame, and by the existing 0.1-second visible-dialog driver so delayed font measurements and UI-scale changes do not leave stale dimensions. Cached measurements avoid rewriting unchanged geometry. Modern clients use the `UI-DiamondDialogBox-Border` and `UI-DialogBox-Background-Dark` atlases and available user-scaled fonts. Clients missing either atlas use the standard dialog backdrop instead. Generic layout and lifecycle tests live in `Widgets/Dialog.test.lua`; consent tests exercise the real library with native-control stand-ins in `Tests/PopupUIHarness.lua`.

- Unanswered consent (`QuestieTrace.settings.dataCollectionConsent == nil`) shows the dialog at login. `/qlt consent` can reopen it at any time.
- Yes grants consent, starts capture if idle, and starts share reminders. An existing active capture is preserved.
- Only an explicit No (`OnCancel` reason `"clicked"`) declines consent and stops/discards any active or unsaved capture. It does not delete previously saved sessions. Replacement and programmatic dismissal have no consent side effects.
- Opening or hiding the frame does not change consent. It has no timeout or Escape-key dismissal, matching the previous prompt.

The consumer calls the private widget's `Show`/`Hide`, never Blizzard popup registration. Frame show/hide and size changes use the coordinator's `Register`, `Unregister`, and `RequestLayout` methods. Do not register it with `StaticPopupDialogs` or `StaticPopupSpecial_Show`: reading addon-owned entries in Blizzard's popup lists can taint subsequent Edit Mode operations.

### Coexisting with Blizzard popups

On show and every 0.1 seconds while visible, the shared stack uses `StaticPopup_ForEachShownDialog` to enumerate normal and special popups, including Edit Mode's new-layout dialog. The callback reads native visibility/rectangle getters and updates local bounds only. Clients without the iterator fall back to scanning `StaticPopup1` through `StaticPopup4`. It positions itself below their combined bounds with a 10-unit gap, or above/right/left if there is not enough room below. With no visible popups it returns to its normal top-center position. If none of the candidate positions fit, it uses the normal position rather than hiding the consent choices; overlap is unavoidable in that case.

Coordinates are converted to the dialog's effective scale and anchored relative to `UIParent`, never to a Blizzard popup. Only the addon-owned frame is repositioned, and only when the destination changes. Secret visibility/coordinates, forbidden frames, or unresolved geometry leave its current placement unchanged. The watcher runs in combat because the dialog is unprotected, and stops naturally when the frame is hidden. It does not cover windows outside Blizzard's shared popup list; the older-client fallback covers normal popups only.

### In-client validation

Lua mocks cannot enforce WoW's taint rules or reproduce native rendering and input. After a clean reload, show the consent dialog, leave it open, enter Edit Mode, create a new layout, and exit Edit Mode. Verify there is no secret-value error and that `PartyFrame.settingMap` and `CompactPartyFrameMember1.optionTable` remain secure. Also check text wrapping and both consent choices; test No only with disposable capture data.

With the consent dialog visible, show/dismiss a normal Blizzard popup and Edit Mode's new-layout dialog and check that ours moves below each and returns afterward. Repeat in combat where the Blizzard dialog is available, and with multiple popups. Check low-screen-space placement and different UI scales without moving or modifying Blizzard's frames.
