QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type l10n
local l10n = Core.l10n

---------------------------------------------------------------------------
-- Settings panel (Game Menu > Options > AddOns > QuestieTrace)
---------------------------------------------------------------------------
-- Holds a checkbox that mirrors `QuestieTrace.settings.dataCollectionConsent` and a button
-- that opens the export window. Checking the box grants consent and unchecking it revokes
-- it, like the answers in the consent prompt. Undecided (nil) shows as unchecked.
---------------------------------------------------------------------------

---@return boolean
local function GetConsent()
  local settings = QuestieTrace and QuestieTrace.settings
  return settings ~= nil and settings.dataCollectionConsent == true
end

---@param granted boolean
local function SetConsent(granted)
  if (granted == GetConsent()) then
    return
  end
  Core.SetConsent(granted)
end

--- Register the settings category. Clients without the Settings API get no panel.
--- Called on PLAYER_LOGIN, once the saved variables are loaded.
function Core.RegisterSettingsPanel()
  if (not (Settings and Settings.RegisterVerticalLayoutCategory and Settings.RegisterProxySetting and Settings.CreateCheckbox)) then
    return
  end

  local category, layout = Settings.RegisterVerticalLayoutCategory("QuestieTrace")
  local setting = Settings.RegisterProxySetting(
    category,
    "QUESTIETRACE_DATA_COLLECTION_CONSENT",
    Settings.VarType.Boolean,
    l10n("Allow data collection"),
    false,
    GetConsent,
    SetConsent
  )
  Settings.CreateCheckbox(
    category,
    setting,
    l10n("Allow QuestieTrace to collect anonymized gameplay data, such as quest progress, positions, and loot. Your data stays on your machine unless you choose to export and share it.")
  )
  if (CreateSettingsButtonInitializer and layout) then
    local addSearchTags = true
    layout:AddInitializer(CreateSettingsButtonInitializer(
      l10n("Export data"),
      l10n("Open export window"),
      function() Core.ShowExportWindow() end,
      l10n("Open the window to review and share the data QuestieTrace has collected."),
      addSearchTags
    ))
  end
  Settings.RegisterAddOnCategory(category)
end
