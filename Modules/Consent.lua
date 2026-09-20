QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type l10n
local l10n = Core.l10n

---------------------------------------------------------------------------
-- Data collection consent
---------------------------------------------------------------------------
-- `QuestieTrace.settings.dataCollectionConsent` is a tri-state value:
--   nil   = not asked yet (first login)
--   true  = user consented to local data collection
--   false = user declined; capture must not start, and we must not ask again
---------------------------------------------------------------------------

if StaticPopupDialogs then
  StaticPopupDialogs["QUESTIETRACE_CONSENT"] = {
    text = l10n(
      "Help improve Questie by allowing QuestieTrace to collect anonymized gameplay data, such as quest progress, positions, and loot. Your data stays on your machine unless you choose to export and share it. Do you want to help out Questie?"
    ),
    button1 = YES or "Yes",
    button2 = NO or "No",
    OnAccept = function()
      QuestieTrace.settings.dataCollectionConsent = true
      Core.PrintConsentReminder()

      -- Start capturing immediately
      if Core.GetCaptureState() == "idle" then
        Core.StartCapture()
      end

      Core.StartShareReminders()
    end,
    OnCancel = function()
      QuestieTrace.settings.dataCollectionConsent = false
      -- Discard collection immediately, including a capture that was
      -- already running before consent was revoked via /qlt consent.
      Core.DiscardCapture()
    end,
    timeout = 0,
    whileDead = 1,
    hideOnEscape = false,
    showAlert = 1,
  }

  StaticPopupDialogs["QUESTIETRACE_CLEAR_ALL"] = {
    -- The %s is filled in by StaticPopup_Show's arg1 (the session count) at
    -- click time, so it has to survive translation here. l10n.translate runs
    -- every key through string.format, which would raise on an unconsumed
    -- %s -- passing the literal "%s" through re-emits the placeholder for
    -- Blizzard to format later, in whichever position the locale puts it.
    text = l10n(
      "Delete ALL %s collected session(s), including data you have not shared yet? This cannot be undone.",
      "%s"
    ),
    button1 = YES or "Yes",
    button2 = NO or "No",
    OnAccept = function()
      -- Resolved at click time, not file-load time: Modules/Consent.lua loads
      -- before QuestieTrace.lua in both TOCs, so Core.ClearAllSessions does
      -- not exist yet while this table is being built.
      ---@type number
      local removed = Core.ClearAllSessions()
      Core.Print(l10n("Deleted %s session(s).", removed))
    end,
    timeout = 0,
    whileDead = 1,
    -- Unlike the consent popup, escaping out of a destructive confirmation is
    -- the safe outcome, so it is allowed here.
    hideOnEscape = 1,
    showAlert = 1,
  }
end

--- Show the consent popup, asking the player for permission to collect data.
function Core.ShowConsentPrompt()
  StaticPopup_Show("QUESTIETRACE_CONSENT")
end

--- Show the `/qlt clear all` confirmation popup.
--- The count is handed to the popup as arg1 so the player sees exactly what
--- they are about to lose. It comes from Core.CountClearableSessions(), the
--- same helper Core.ClearAllSessions() reports from, so the number shown here
--- and the number reported afterwards can never disagree.
function Core.ShowClearAllPrompt()
  StaticPopup_Show("QUESTIETRACE_CLEAR_ALL", Core.CountClearableSessions())
end

--- Print a friendly reminder to chat that gameplay data is being collected locally.
--- Only meaningful (and only ever called) when consent has been granted.
function Core.PrintConsentReminder()
  Core.Print(l10n("Thank you for helping improve Questie! Your gameplay data is being collected locally on this device."))
end
