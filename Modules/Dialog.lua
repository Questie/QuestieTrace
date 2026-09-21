-- luacheck: globals C_Texture BACKDROP_DIALOG_32_32 GameFontDisable
-- luacheck: globals UserScaledFontGameHighlight UserScaledFontGameNormal UserScaledFontGameDisable

---@type QuestieTraceCore
local Core = QuestieTraceCore

---@class QuestieTraceDialog : Frame
---@field Background Texture
---@field Border Texture
---@field Text FontString
---@field AcceptButton Button
---@field DeclineButton Button

--- Style only our own controls. Inheriting StaticPopupTemplate would register
--- this frame with Blizzard's popup manager and reintroduce the taint path.
---@param frame QuestieTraceDialog
function Core.OnDialogLoad(frame)
  local backgroundAtlas = "UI-DialogBox-Background-Dark"
  local borderAtlas = "UI-DiamondDialogBox-Border"
  local getAtlasInfo = C_Texture and C_Texture.GetAtlasInfo
  if getAtlasInfo and getAtlasInfo(backgroundAtlas) and getAtlasInfo(borderAtlas) then
    frame:SetBackdrop(nil)
    frame.Background:SetAtlas(backgroundAtlas)
    frame.Border:SetAtlas(borderAtlas)
    frame.Background:Show()
    frame.Border:Show()
  else
    -- Classic clients may not have the modern dialog atlases.
    frame.Background:Hide()
    frame.Border:Hide()
    frame:SetBackdrop(BACKDROP_DIALOG_32_32)
  end

  frame.Text:SetFontObject(UserScaledFontGameHighlight or GameFontHighlight)
  for _, button in ipairs({ frame.AcceptButton, frame.DeclineButton }) do
    button:SetNormalFontObject(UserScaledFontGameNormal or GameFontNormal)
    button:SetHighlightFontObject(UserScaledFontGameHighlight or GameFontHighlight)
    button:SetDisabledFontObject(UserScaledFontGameDisable or GameFontDisable)
  end
end

--- Match GameDialog's default spacing, growing to fit translated button labels
--- and wrapped text. Font objects are shared read-only; only our widgets resize.
---@param frame QuestieTraceDialog
function Core.LayoutDialog(frame)
  local buttonWidth = math.max(120, frame.AcceptButton:GetTextWidth() + 20, frame.DeclineButton:GetTextWidth() + 20)
  local buttonHeight = math.max(21,
    frame.AcceptButton:GetFontString():GetStringHeight() + 8,
    frame.DeclineButton:GetFontString():GetStringHeight() + 8)
  frame.AcceptButton:SetSize(buttonWidth, buttonHeight)
  frame.DeclineButton:SetSize(buttonWidth, buttonHeight)

  -- XML uses 16 above the text, 9 before the buttons, and 16 below them.
  frame:SetWidth(math.max(420, buttonWidth * 2 + 10 + 32))
  frame:SetHeight(16 + frame.Text:GetStringHeight() + 9 + buttonHeight + 16)
end
