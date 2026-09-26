local _, addon = ...
local Dialogs = addon.Dialog

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

local CONSENT_DIALOG = "QUESTIETRACE_DATA_COLLECTION_CONSENT"

Dialogs.Dialogs[CONSENT_DIALOG] = {
  text = l10n(
    "Help improve Questie by allowing QuestieTrace to collect anonymized gameplay data, such as quest progress, positions, and loot. Your data stays on your machine unless you choose to export and share it. Do you want to help out Questie?"
  ),
  button1 = YES or "Yes",
  button2 = NO or "No",
  showAlert = true,
  whileDead = true,
  hideOnEscape = false,
  noCancelOnReuse = true,
  OnAccept = function()
    QuestieTrace.settings.dataCollectionConsent = true
    Core.PrintConsentReminder()
    if Core.GetCaptureState() == "idle" then
      Core.StartCapture()
    end
    Core.StartShareReminders()
  end,
  OnCancel = function(_, _, reason)
    -- Only an explicit No revokes consent; replacement or dismissal is not a choice.
    if reason ~= "clicked" then return end
    QuestieTrace.settings.dataCollectionConsent = false
    Core.DiscardCapture()
  end,
}

--- Ask for consent without entering Blizzard's shared StaticPopup lists.
--- Repeated requests preserve the visible decision rather than replacing it.
---@return DialogFrame?
function Core.ShowConsentPrompt()
  return Dialogs.FindVisible(CONSENT_DIALOG) or Dialogs.Show(CONSENT_DIALOG)
end

--- Print a friendly reminder to chat that gameplay data is being collected locally.
--- Only meaningful (and only ever called) when consent has been granted.
function Core.PrintConsentReminder()
  Core.Print(l10n("Thank you for helping improve Questie! Your gameplay data is being collected locally on this device."))
end
