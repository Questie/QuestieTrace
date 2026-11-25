---@type QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- Casts on world objects
---------------------------------------------------------------------------
-- Opening, mining, gathering and most other ways to use a world object are spells the player casts on
-- it, and UNIT_SPELLCAST_SENT has the object's name as the cast's target. That names objects a click
-- uses up without a loot window, which no GUID is ever given for.
--
-- ObjectCast (synthetic) -> { spellID = number, target = string }: the spell and the name of the object
--   (or, for a spell that opens both, item) it was cast on. One entry per cast attempt: the event fires
--   when the cast is sent, whether it succeeds or not.
--
-- Privacy: the target of a heal or buff is a player's name. Only the spells of Core.OBJECT_SPELLS are
-- read, whose own data says that they target objects and no unit other than the player, and the name
-- goes through SanitizeText. The event is not recorded itself: it has the names of every other cast.
---------------------------------------------------------------------------

---@type CaptureState?
local captureRef
---@type FunctionStreamEntry[]?
local castStream

---@param value any
---@param valueType "number"|"string"
---@return any value nil when it is restricted (secret) or of another type
local function Readable(value, valueType)
  if issecretvalue and issecretvalue(value) then return nil end
  return type(value) == valueType and value or nil
end

---@param unit any
---@param target any
---@param spellID any
local function RecordCast(unit, target, spellID)
  local capture = captureRef
  if (not capture) or (not castStream) then return end
  if Readable(unit, "string") ~= "player" then return end

  local castSpellID = Readable(spellID, "number")
  if (not castSpellID) or (not Core.OBJECT_SPELLS[castSpellID]) then return end
  local name = Readable(target, "string")
  if (not name) or name == "" then return end

  castStream[#castStream + 1] = {
    t = GetTime() - capture.startedAt,
    tp = GetTimePreciseSec() - capture.startedAtPrecise,
    v = { spellID = castSpellID, target = Core.SanitizeText(name) },
  }
end

-- A private frame, as tracked events are recorded with their arguments.
---@type Frame
local castFrame = CreateFrame("Frame")
castFrame:SetScript("OnEvent", function(_, _, unit, target, _, spellID)
  RecordCast(unit, target, spellID)
end)

Core.RegisterTracker({
  ---@param capture CaptureState
  Init = function(capture)
    castStream = {}
    capture.session.functions["ObjectCast"] = castStream
    captureRef = capture
    castFrame:RegisterEvent("UNIT_SPELLCAST_SENT")
  end,

  OnCaptureStopped = function()
    captureRef, castStream = nil, nil
    castFrame:UnregisterEvent("UNIT_SPELLCAST_SENT")
  end,
})
