QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type l10n
local l10n = Core.l10n

---------------------------------------------------------------------------
-- Export window (UI only -- calls into Export/Export.lua for data)
---------------------------------------------------------------------------

---@class ExportFrame : Frame
---@field editBox EditBox
---@field scrollFrame ScrollFrame
---@field urlEditBox EditBox
---@field hint FontString
---@field warningText FontString
---@field reportedButton Button

---@type ExportFrame?
local exportFrame

--- Source sessions backing the export string currently shown in
--- exportFrame.editBox (second return value of Core.BuildExportPayload()).
--- Set (to a value, or explicitly to nil) on every Core.ShowExportWindow()
--- call, and cleared once the player confirms via the "I reported this"
--- button. Not deleted until that explicit confirmation happens, so merely
--- opening or closing the window can never make the data unrecoverable.
---@type SessionRecord[]?
local pendingSourceSessions

--- Build the export window frame (lazy; created on first use).
---@return ExportFrame
local function BuildExportFrame()
  local frame = CreateFrame("Frame", "QuestieTraceExportFrame", UIParent) --[[@as ExportFrame]]
  frame:SetSize(520, 420)
  frame:SetPoint("CENTER")
  frame:SetFrameStrata("DIALOG")
  frame:SetClampedToScreen(true)
  frame:SetMovable(true)
  frame:EnableMouse(true)
  frame:RegisterForDrag("LeftButton")
  frame:SetScript("OnDragStart", function(self) self:StartMoving() end)
  frame:SetScript("OnDragStop", function(self) self:StopMovingOrSizing() end)
  frame:Hide()

  local bg = frame:CreateTexture(nil, "BACKGROUND")
  bg:SetAllPoints()
  bg:SetColorTexture(0.05, 0.05, 0.05, 0.95)

  local border = frame:CreateTexture(nil, "BORDER")
  border:SetAllPoints()
  border:SetColorTexture(0.20, 0.20, 0.20, 0.9)

  local inner = frame:CreateTexture(nil, "ARTWORK")
  inner:SetPoint("TOPLEFT", 1, -1)
  inner:SetPoint("BOTTOMRIGHT", -1, 1)
  inner:SetColorTexture(0.08, 0.08, 0.08, 0.9)

  local title = frame:CreateFontString(nil, "OVERLAY", "GameFontNormal")
  title:SetPoint("TOP", 0, -12)
  title:SetText(l10n("QuestieTrace Export"))

  local urlEditBox = CreateFrame("EditBox", nil, frame, "InputBoxTemplate") --[[@as EditBox]]
  urlEditBox:SetSize(470, 20)
  urlEditBox:SetPoint("TOP", title, "BOTTOM", 0, -10)
  urlEditBox:SetAutoFocus(false)
  urlEditBox:SetFontObject("ChatFontNormal")
  urlEditBox:SetText("https://questie.dev/trace")
  urlEditBox:SetScript("OnEscapePressed", function(self) self:ClearFocus() end)
  urlEditBox:SetScript("OnCursorChanged", function(self)
    self:SetText("https://questie.dev/trace")
    self:HighlightText()
  end)
  urlEditBox:SetScript("OnEditFocusGained", function(self) self:HighlightText() end)

  local hint = frame:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
  hint:SetPoint("TOP", urlEditBox, "BOTTOM", 0, -10)
  hint:SetPoint("LEFT", 16, 0)
  hint:SetPoint("RIGHT", -16, 0)
  hint:SetJustifyH("CENTER")
  hint:SetText(l10n("Copy the text below and submit it at the URL above. Please do this again each time this window has new data to share, since one submission only covers what happened up to that point. This helps us build the Questie database. Player and guild names are never included."))

  local scrollFrame = CreateFrame("ScrollFrame", nil, frame, "UIPanelScrollFrameTemplate") --[[@as ScrollFrame]]
  scrollFrame:SetPoint("TOPLEFT", 16, -110)
  scrollFrame:SetPoint("BOTTOMRIGHT", -32, 44)

  local editBox = CreateFrame("EditBox", nil, scrollFrame) --[[@as EditBox]]
  editBox:SetMultiLine(true)
  editBox:SetFontObject("ChatFontNormal")
  editBox:SetWidth(456)
  editBox:SetAutoFocus(false)
  editBox:SetScript("OnEscapePressed", function(self) self:ClearFocus() end)

  --- Make the edit box read-only by reverting text on cursor change (any edit attempt).
  editBox:SetScript("OnCursorChanged", function(self)
    self:SetText(self.originalText)
    self:HighlightText()
  end)
  scrollFrame:SetScrollChild(editBox)

  local warningText = frame:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
  warningText:SetPoint("CENTER")
  warningText:SetJustifyH("CENTER")
  warningText:SetMaxLines(0)
  warningText:SetWordWrap(true)
  warningText:SetWidth(456)
  warningText:Hide()

  local closeButton = CreateFrame("Button", nil, frame, "UIPanelButtonTemplate") --[[@as Button]]
  closeButton:SetSize(100, 22)
  closeButton:SetPoint("BOTTOMLEFT", 16, 12)
  closeButton:SetText(l10n("Close"))
  closeButton:SetScript("OnClick", function() frame:Hide() end)
  closeButton:SetScript("OnEnter", function(self)
    GameTooltip:SetOwner(self, "ANCHOR_TOP")
    GameTooltip:SetText(l10n("Closes this window without marking the data as reported. You can reopen it later to submit the same data."), nil, nil, nil, nil, true)
    GameTooltip:Show()
  end)
  closeButton:SetScript("OnLeave", function() GameTooltip:Hide() end)

  local reportedButton = CreateFrame("Button", nil, frame, "UIPanelButtonTemplate") --[[@as Button]]
  reportedButton:SetSize(150, 22)
  reportedButton:SetPoint("BOTTOMRIGHT", -16, 12)
  reportedButton:SetText(l10n("I reported this"))
  reportedButton:SetScript("OnClick", function() Core.ConfirmExportReported() end)
  reportedButton:SetScript("OnEnter", function(self)
    GameTooltip:SetOwner(self, "ANCHOR_TOP")
    GameTooltip:SetText(l10n("Only click this after you have copied the text above and submitted it at the URL. This marks the data as reported so it will not be shown again."), nil, nil, nil, nil, true)
    GameTooltip:Show()
  end)
  reportedButton:SetScript("OnLeave", function() GameTooltip:Hide() end)

  frame.editBox = editBox
  frame.scrollFrame = scrollFrame
  frame.urlEditBox = urlEditBox
  frame.hint = hint
  frame.warningText = warningText
  frame.reportedButton = reportedButton
  return frame
end

--- Confirm that the data currently shown in the export window has actually
--- been copied and submitted at the URL. This is the only path that deletes
--- the reported sessions -- merely opening or closing the window never does,
--- so a player who dismisses the window without submitting can always get
--- the same data back later. No-op if there is nothing pending (window never
--- shown, no exportable data, or the codec is unavailable).
function Core.ConfirmExportReported()
  if not pendingSourceSessions then
    return
  end

  Core.DeleteReportedSessions(pendingSourceSessions)
  pendingSourceSessions = nil

  if exportFrame then
    exportFrame:Hide()
    GameTooltip:Hide()
  end
end

--- Show the export window, populated with the current export string.
--- If there is nothing to export, displays a warning and hides the input elements.
function Core.ShowExportWindow()
  if (not exportFrame) then
    exportFrame = BuildExportFrame()
  end

  -- Build once: reused for the hasExportableData check, the encoded string,
  -- and deleting reported sessions, so all three agree on the same data.
  local payload, sourceSessions = Core.BuildExportPayload()
  local hasData = payload.hasExportableData

  if hasData then
    -- Show the export data
    local ok, text = Core.BuildExportString(payload)
    exportFrame.editBox.originalText = text
    exportFrame.editBox:SetText(text)

    -- Show export UI, hide warning
    exportFrame.urlEditBox:Show()
    exportFrame.hint:Show()
    exportFrame.scrollFrame:Show()
    exportFrame.warningText:Hide()
    exportFrame.editBox:SetFocus()

    -- Only offer "I reported this" if the string actually contains encoded
    -- data. If the codec is unavailable, `text` is just an error message --
    -- there is nothing real to confirm as reported.
    if ok then
      pendingSourceSessions = sourceSessions
      exportFrame.reportedButton:Show()
    else
      pendingSourceSessions = nil
      exportFrame.reportedButton:Hide()
    end
  else
    -- Show warning, hide export data
    exportFrame.warningText:SetText(l10n("Nothing new to share yet. Keep playing and check back later."))
    exportFrame.warningText:Show()

    exportFrame.urlEditBox:Hide()
    exportFrame.hint:Hide()
    exportFrame.scrollFrame:Hide()
    exportFrame.reportedButton:Hide()
    pendingSourceSessions = nil
  end

  exportFrame:Show()

  -- Pause share reminders until another session is saved. The write itself
  -- lives in ExportReminder.lua so this file stays free of SavedVariables access.
  Core.MarkExportOpened()
end
