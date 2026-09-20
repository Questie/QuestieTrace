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

---@type ExportFrame?
local exportFrame

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
  hint:SetText(l10n("Copy the text below and submit it at the URL above. This data helps us build the Questie database. Player and guild names are never included."))

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
  closeButton:SetSize(80, 22)
  closeButton:SetPoint("BOTTOM", 0, 12)
  closeButton:SetText(l10n("Close"))
  closeButton:SetScript("OnClick", function() frame:Hide() end)

  frame.editBox = editBox
  frame.scrollFrame = scrollFrame
  frame.urlEditBox = urlEditBox
  frame.hint = hint
  frame.warningText = warningText
  return frame
end

--- Show the export window, populated with the current export string.
--- If there is nothing to export, displays a warning and hides the input elements.
---
--- By default, sessions already shown in a previous export are left out so
--- the same data is never bundled twice. Pass includeAlreadyExported = true
--- to force everything back in (e.g. to resend after a failed submission).
---@param includeAlreadyExported boolean?
function Core.ShowExportWindow(includeAlreadyExported)
  if (not exportFrame) then
    exportFrame = BuildExportFrame()
  end

  -- Build once: reused for the hasExportableData check, the encoded string,
  -- and marking sessions exported, so all three agree on the same data.
  local payload, sourceSessions = Core.BuildExportPayload(includeAlreadyExported)
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

    -- Only treat this as "shown to the player" if the string actually
    -- contains encoded data. If the codec is unavailable, `text` is just an
    -- error message -- marking sessions exported here would make that data
    -- unrecoverable since it would never be offered again.
    if ok then
      -- Sweep sessions left over from the *previous* export cycle now that we
      -- know this cycle produced a usable payload. This must stay inside the
      -- `ok` branch: if the codec were unavailable, sweeping here would
      -- destroy the prior export's recoverable data while handing the user
      -- nothing new in its place. It must also run before
      -- Core.MarkSessionsExported below so it only ever removes sessions
      -- exported by an earlier cycle, never the ones just bundled into
      -- `sourceSessions` -- Core.BuildExportPayload already excludes
      -- already-exported sessions, so the two calls can never target the
      -- same rows.
      --
      -- Skipped entirely when includeAlreadyExported is true (`/qlt export
      -- all`): that flag exists specifically to resend sessions that were
      -- already marked exported, so sweeping here would delete the exact
      -- data the user is trying to recover.
      if not includeAlreadyExported then
        Core.ClearExportedSessions()
      end

      Core.MarkSessionsExported(sourceSessions)

      -- Finalize and restart the live session (if it was exported) so new events
      -- don't get added to an already-exported record.
      Core.FinalizeLiveSessionIfExported()
    end
  else
    -- Show warning, hide export data
    exportFrame.warningText:SetText(l10n("Nothing new to share yet. Keep playing and check back later."))
    exportFrame.warningText:Show()

    exportFrame.urlEditBox:Hide()
    exportFrame.hint:Hide()
    exportFrame.scrollFrame:Hide()
  end

  exportFrame:Show()

  -- Pause share reminders until another session is saved. The write itself
  -- lives in ExportReminder.lua so this file stays free of SavedVariables access.
  Core.MarkExportOpened()
end
