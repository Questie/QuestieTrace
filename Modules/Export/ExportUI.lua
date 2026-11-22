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
---@field urlEditBox EditBox
---@field hint FontString
---@field warningText FontString
---@field statsFrame Frame Container for every statistic, shown/hidden as one.
---@field stats ExportStatRows
---@field reportedButton Button

---@class ExportStatRow
---@field icon Texture
---@field text FontString

---@class ExportStatRows
---@field events ExportStatRow
---@field time ExportStatRow
---@field questsAccepted ExportStatRow
---@field questsTurnedIn ExportStatRow

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

--- Coroutine currently filling the export window, if any (see Core.ShowExportWindow()).
---@type thread?
local exportJob

--- Icons shown before each statistic. All ship with the Classic client.
---@type string
local ICON_EVENTS = "Interface\\Icons\\INV_Misc_Note_02"
---@type string
local ICON_TIME = "Interface\\Icons\\INV_Misc_PocketWatch_01"
---@type string
local ICON_QUEST_ACCEPTED = "Interface\\GossipFrame\\AvailableQuestIcon"
---@type string
local ICON_QUEST_TURNED_IN = "Interface\\GossipFrame\\ActiveQuestIcon"

---@type integer Icon width/height in pixels.
local STATS_ICON_SIZE = 16
---@type integer Gap between an icon and its text.
local STATS_ICON_GAP = 4
---@type integer Width of each of the two statistics columns.
local STATS_COLUMN_WIDTH = 225
---@type integer Horizontal gap between the two columns.
local STATS_COLUMN_GAP = 10
---@type integer Vertical distance from one row's top to the next.
local STATS_ROW_HEIGHT = 22
---@type string Gold, matching NORMAL_FONT_COLOR, so values stand out from the white labels.
local STATS_VALUE_COLOR = "|cffffd100"

--- Color a statistic's value so it stands out from its label.
---@param value string
---@return string
local function _HighlightValue(value)
  return STATS_VALUE_COLOR .. value .. "|r"
end

--- Create one statistic: an icon with its text beside it.
---
--- The text is anchored LEFT to the icon's RIGHT, which centers it vertically
--- on the icon. (An inline |T|t icon inside the text sits on the text
--- baseline instead, so a 16 px icon next to smaller text looks misaligned.)
--- The text has a fixed width without wrapping, so a long translation is
--- cut short instead of running into the other column.
---@param parent Frame
---@param iconPath string
---@param column integer 0 = left, 1 = right.
---@param row integer 0 = top.
---@return ExportStatRow
local function _CreateStatRow(parent, iconPath, column, row)
  local icon = parent:CreateTexture(nil, "ARTWORK")
  icon:SetSize(STATS_ICON_SIZE, STATS_ICON_SIZE)
  icon:SetPoint("TOPLEFT", parent, "TOPLEFT", column * (STATS_COLUMN_WIDTH + STATS_COLUMN_GAP), -row * STATS_ROW_HEIGHT)
  icon:SetTexture(iconPath)

  local text = parent:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
  text:SetPoint("LEFT", icon, "RIGHT", STATS_ICON_GAP, 0)
  text:SetWidth(STATS_COLUMN_WIDTH - STATS_ICON_SIZE - STATS_ICON_GAP)
  text:SetJustifyH("LEFT")
  text:SetWordWrap(false)

  return { icon = icon, text = text }
end

--- Format a count for display, with the client's thousands separators when
--- available. BreakUpLargeNumbers is looked up on every call, not cached.
---@param count integer
---@return string
local function _FormatCount(count)
  if type(BreakUpLargeNumbers) == "function" then
    return BreakUpLargeNumbers(count)
  end
  return tostring(count)
end

--- Format a duration for display, using the client's localized time text
--- (e.g. "3 Hr 12 Min") when available. SecondsToTime is looked up on every
--- call, not cached, and returns "" for under a second, so that falls back too.
---@param seconds number
---@return string
local function _FormatDuration(seconds)
  if type(SecondsToTime) == "function" then
    local text = SecondsToTime(seconds)
    if text ~= "" then
      return text
    end
  end
  local whole = math.floor(seconds)
  return string.format("%d:%02d:%02d", math.floor(whole / 3600), math.floor(whole % 3600 / 60), whole % 60)
end

--- Build the export window frame (lazy; created on first use).
---@return ExportFrame
local function BuildExportFrame()
  local frame = CreateFrame("Frame", "QuestieTraceExportFrame", UIParent) --[[@as ExportFrame]]
  frame:SetSize(500, 270)
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
  urlEditBox:SetSize(410, 20)
  urlEditBox:SetPoint("TOP", title, "BOTTOM", 0, -10)
  urlEditBox:SetAutoFocus(false)
  urlEditBox:SetFontObject("ChatFontNormal")
  urlEditBox:SetText("https://questie.dev/trace")
  urlEditBox:SetScript("OnEscapePressed", function(self) self:ClearFocus() end)
  urlEditBox:SetScript("OnTextChanged", function(self, userInput)
    if userInput then
      self:SetText("https://questie.dev/trace")
      self:HighlightText()
    end
  end)

  local hint = frame:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
  hint:SetPoint("TOP", urlEditBox, "BOTTOM", 0, -10)
  hint:SetPoint("LEFT", 16, 0)
  hint:SetPoint("RIGHT", -16, 0)
  hint:SetJustifyH("CENTER")
  hint:SetText(l10n("Copy the text below and submit it at the URL above. Please do this again each time this window has new data to share, since one submission only covers what happened up to that point. This helps us build the Questie database. Player and guild names are never included."))

  --- Single-line on purpose: the client fails to render (and stalls on) very
  --- large export strings in a multiline edit box, while a single-line box only
  --- draws the visible part. Ctrl+A / Ctrl+C still copy the full string.
  local editBox = CreateFrame("EditBox", nil, frame, "InputBoxTemplate") --[[@as EditBox]]
  editBox:SetSize(410, 20)
  editBox:SetPoint("TOP", hint, "BOTTOM", 0, -10)
  editBox:SetMaxLetters(0)
  editBox:SetFontObject("ChatFontNormal")
  editBox:SetAutoFocus(false)
  editBox:SetScript("OnEscapePressed", function(self) self:ClearFocus() end)

  --- Make the edit box read-only by reverting only real user edits. Reacting to
  --- every cursor move instead would re-set the whole (potentially huge) export
  --- string on each click or arrow key.
  editBox:SetScript("OnTextChanged", function(self, userInput)
    if userInput then
      self:SetText(self.originalText)
      self:HighlightText()
    end
  end)
  editBox:SetScript("OnEditFocusGained", function(self) self:HighlightText() end)
  editBox:SetScript("OnMouseUp", function(self) self:HighlightText() end)

  --- Two columns: activity on the left, quests on the right.
  local statsFrame = CreateFrame("Frame", nil, frame) --[[@as Frame]]
  statsFrame:SetSize(STATS_COLUMN_WIDTH * 2 + STATS_COLUMN_GAP, STATS_ROW_HEIGHT + STATS_ICON_SIZE)
  statsFrame:SetPoint("TOP", editBox, "BOTTOM", 0, -12)
  statsFrame:Hide()

  ---@type ExportStatRows
  local stats = {
    events = _CreateStatRow(statsFrame, ICON_EVENTS, 0, 0),
    time = _CreateStatRow(statsFrame, ICON_TIME, 0, 1),
    questsAccepted = _CreateStatRow(statsFrame, ICON_QUEST_ACCEPTED, 1, 0),
    questsTurnedIn = _CreateStatRow(statsFrame, ICON_QUEST_TURNED_IN, 1, 1),
  }

  local warningText = frame:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
  warningText:SetPoint("CENTER")
  warningText:SetJustifyH("CENTER")
  warningText:SetMaxLines(0)
  warningText:SetWordWrap(true)
  warningText:SetWidth(380)
  warningText:Hide()

  local closeButton = CreateFrame("Button", nil, frame, "UIPanelButtonTemplate") --[[@as Button]]
  closeButton:SetSize(100, 22)
  closeButton:SetPoint("BOTTOMRIGHT", -16, 12)
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
  reportedButton:SetPoint("BOTTOM", 0, 12)
  reportedButton:SetText(l10n("I reported this"))
  reportedButton:SetScript("OnClick", function() Core.ConfirmExportReported() end)
  reportedButton:SetScript("OnEnter", function(self)
    GameTooltip:SetOwner(self, "ANCHOR_TOP")
    GameTooltip:SetText(l10n("Only click this after you have copied the text above and submitted it at the URL. This marks the data as reported so it will not be shown again."), nil, nil, nil, nil, true)
    GameTooltip:Show()
  end)
  reportedButton:SetScript("OnLeave", function() GameTooltip:Hide() end)

  frame.editBox = editBox
  frame.urlEditBox = urlEditBox
  frame.hint = hint
  frame.warningText = warningText
  frame.statsFrame = statsFrame
  frame.stats = stats
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

--- Hide the statistics and the "I reported this" button, and forget the pending
--- sessions, for when there is no valid export string to report.
---@param frame ExportFrame
local function _HideReportingControls(frame)
  pendingSourceSessions = nil
  frame.statsFrame:Hide()
  frame.reportedButton:Hide()
end

--- Show "nothing to share" instead of the export string.
---@param frame ExportFrame
local function _ShowNothingToShare(frame)
  frame.warningText:SetText(l10n("Nothing new to share yet. Keep playing and check back later."))
  frame.warningText:Show()

  frame.urlEditBox:Hide()
  frame.hint:Hide()
  frame.editBox:Hide()
  _HideReportingControls(frame)
end

--- Show the "preparing" message while the export job runs. Nothing can be
--- copied or confirmed until the job has finished.
---@param frame ExportFrame
local function _ShowPreparing(frame)
  frame.warningText:SetText(l10n("Preparing export..."))
  frame.warningText:Show()

  frame.urlEditBox:Hide()
  frame.hint:Hide()
  frame.editBox:Hide()
  _HideReportingControls(frame)
end

--- Save the live session and, if it was recording, keep recording in a new
--- one, so later events cannot change what is exported. Skipped without codec
--- support, because saving would then drop the session.
---
--- The saved sessions are listed before saving, because saving may prune the
--- oldest one, which must still be exported.
---@return EncodedSessionRecord[] savedSessions Saved sessions to export, including the live session if it was saved.
---@return boolean skipLiveSession True if the current live session was started after the export began.
local function _RotateLiveSession()
  local characterDb = type(QuestieTraceCharacter) == "table" and QuestieTraceCharacter or {}
  local storedSessions = type(characterDb.sessions) == "table" and characterDb.sessions or {}
  ---@type EncodedSessionRecord[]
  local savedSessions = {}
  for i = 1, #storedSessions do
    savedSessions[i] = storedSessions[i]
  end

  local liveSession = characterDb.currentSession
  if type(liveSession) ~= "table" or type(liveSession.events) ~= "table" or #liveSession.events == 0 then
    return savedSessions, false
  end
  if (not Core.HasCodecSupport()) then
    return savedSessions, false
  end

  local wasRunning = Core.GetCaptureState() == "running"
  local lastBefore = storedSessions[#storedSessions]
  Core.SaveCapture()
  -- Only continue recording if the player was recording; a stopped capture stays stopped.
  if wasRunning then
    Core.StartCapture()
  end

  -- SaveCapture appends the live session as a new record.
  storedSessions = type(characterDb.sessions) == "table" and characterDb.sessions or {}
  local lastAfter = storedSessions[#storedSessions]
  if lastAfter ~= nil and lastAfter ~= lastBefore then
    savedSessions[#savedSessions + 1] = lastAfter
  end
  -- If saving left the live session in place, it is still the one to export,
  -- and confirming discards it (see Core.DeleteReportedSessions()).
  return savedSessions, characterDb.currentSession ~= liveSession
end

--- Fill the export window. Runs as a coroutine and yields between the slow
--- steps (saving the live session, decoding each saved session, compressing,
--- and each chunk of turning it into printable text),
--- so no single frame runs long enough to hit the client's script time limit.
---@param frame ExportFrame
local function _RunExportJob(frame)
  local savedSessions, skipLiveSession = _RotateLiveSession()
  -- Build once: reused for the hasExportableData check, the encoded string,
  -- and deleting reported sessions, so all three agree on the same data.
  local payload, sourceSessions = Core.BuildExportPayload({
    savedSessions = savedSessions,
    skipLiveSession = skipLiveSession,
    yield = coroutine.yield,
  })
  if (not payload.hasExportableData) then
    _ShowNothingToShare(frame)
    return
  end

  coroutine.yield()
  local ok, text = Core.BuildExportString(payload, coroutine.yield)
  frame.editBox.originalText = text
  frame.editBox:SetText(text)

  -- Show export UI, hide warning
  frame.urlEditBox:Show()
  frame.hint:Show()
  frame.editBox:Show()
  frame.warningText:Hide()
  frame.editBox:SetFocus()
  frame.editBox:HighlightText()

  -- Only offer "I reported this" if the string actually contains encoded
  -- data. If the codec is unavailable, `text` is just an error message --
  -- there is nothing real to confirm as reported.
  if (not ok) then
    _HideReportingControls(frame)
    return
  end

  -- Counted from the same payload that was just encoded, so the numbers
  -- match exactly what the player is about to submit.
  local stats = Core.BuildExportStats(payload)
  local rows = frame.stats
  rows.events.text:SetText(l10n("Events captured: %s", _HighlightValue(_FormatCount(stats.eventCount))))
  rows.time.text:SetText(l10n("Time recorded: %s", _HighlightValue(_FormatDuration(stats.timeRecorded))))
  rows.questsAccepted.text:SetText(l10n("Quests accepted: %s", _HighlightValue(_FormatCount(stats.questsAccepted))))
  rows.questsTurnedIn.text:SetText(l10n("Quests turned in: %s", _HighlightValue(_FormatCount(stats.questsTurnedIn))))
  frame.statsFrame:Show()

  pendingSourceSessions = sourceSessions
  frame.reportedButton:Show()
end

--- Run the next step of `job`, then schedule the following one for the next
--- frame. Stops when the job is done, replaced by a newer one, or the window
--- was closed.
---@param job thread
local function _StepExportJob(job)
  if job ~= exportJob then
    return
  end
  if (not exportFrame) or (not exportFrame:IsShown()) then
    exportJob = nil
    return
  end

  local ok, err = coroutine.resume(job)
  if (not ok) or coroutine.status(job) == "dead" then
    exportJob = nil
  end
  if (not ok) then
    error(err, 0)
  end

  if exportJob == job then
    C_Timer.After(0, function() _StepExportJob(job) end)
  end
end

--- Show the export window and fill it with the current export string over
--- the next frames. Reopening the window restarts the job; closing it stops it.
---
--- Before the payload is built, the live session (if any) is saved, and if a
--- capture was running a new session is started, so later events don't change
--- the exported data. This guarantees that DeleteReportedSessions() will
--- delete only the exact data that was shown, not a version polluted by later events.
function Core.ShowExportWindow()
  if (not exportFrame) then
    exportFrame = BuildExportFrame()
  end

  _ShowPreparing(exportFrame)
  exportFrame:Show()

  local frame = exportFrame
  local job = coroutine.create(function() _RunExportJob(frame) end)
  exportJob = job
  _StepExportJob(job)

  -- Pause share reminders until another session is saved. The write itself
  -- lives in ExportReminder.lua so this file stays free of SavedVariables access.
  Core.MarkExportOpened()
end
