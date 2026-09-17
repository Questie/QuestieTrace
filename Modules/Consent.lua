QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type l10n
local l10n = Core.l10n

---@type string
local ADDON_NAME = "QuestieTrace"

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
end

--- Show the consent popup, asking the player for permission to collect data.
function Core.ShowConsentPrompt()
  StaticPopup_Show("QUESTIETRACE_CONSENT")
end

--- Print a friendly reminder to chat that gameplay data is being collected locally.
--- Only meaningful (and only ever called) when consent has been granted.
function Core.PrintConsentReminder()
  print(ADDON_NAME, l10n("Thank you for helping improve Questie! Your gameplay data is being collected locally on this device."))
end
