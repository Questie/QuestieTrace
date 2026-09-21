-- These mocks check placement math and polling, not WoW's native taint rules.
local function RectFrame(left, bottom, width, height, scale)
  return {
    left = left, bottom = bottom, width = width, height = height, scale = scale or 1,
    visible = true,
    rectReads = 0,
    IsVisible = function(self) return self.visible end,
    IsForbidden = function(self) return self.forbidden or false end,
    GetEffectiveScale = function(self) return self.scale end,
    GetRect = function(self)
      self.rectReads = self.rectReads + 1
      return self.left, self.bottom, self.width, self.height
    end,
  }
end

describe("dialog popup avoidance", function()
  local env, core, frame

  before_each(function()
    env = { QuestieTraceCore = {}, UIParent = RectFrame(0, 0, 1000, 800) }
    setmetatable(env, { __index = _G })
    env._G = env
    local chunk = assert(loadfile("Modules/Dialog.lua"))
    setfenv(chunk, env)
    chunk()
    core = env.QuestieTraceCore
    frame = {
      scale = 1, moves = 0,
      GetWidth = function() return 420 end,
      GetHeight = function() return 140 end,
      GetEffectiveScale = function(self) return self.scale end,
      ClearAllPoints = function() end,
      SetPoint = function(self, point, relative, relativePoint, x, y)
        assert.equals("TOP", point)
        assert.equals(env.UIParent, relative)
        assert.equals("BOTTOMLEFT", relativePoint)
        self.x, self.y = x, y
        self.moves = self.moves + 1
      end,
    }
  end)

  it("polls every 0.2 seconds and only moves when the destination changes", function()
    env.StaticPopup1 = RectFrame(290, 530, 420, 135)
    core.OnDialogUpdate(frame, 0.1)
    assert.equals(0, env.StaticPopup1.rectReads)
    assert.equals(0, frame.moves)

    core.OnDialogUpdate(frame, 0.1)
    assert.equals(1, env.StaticPopup1.rectReads)
    assert.equals(500, frame.x)
    assert.equals(520, frame.y)
    assert.equals(1, frame.moves)

    core.OnDialogUpdate(frame, 0.2)
    assert.equals(2, env.StaticPopup1.rectReads)
    assert.equals(1, frame.moves)
  end)

  it("follows the lowest visible popup and restores the normal position", function()
    env.StaticPopup1 = RectFrame(290, 530, 420, 135)
    env.StaticPopup4 = RectFrame(290, 385, 420, 135)
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(375, frame.y)

    env.StaticPopup4.visible = false
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(520, frame.y)
    assert.equals(1, env.StaticPopup4.rectReads)

    env.StaticPopup1.visible = false
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(500, frame.x)
    assert.equals(665, frame.y)
  end)

  it("uses Blizzard's iterator to include special dialogs alongside normal popups", function()
    env.StaticPopup1 = RectFrame(290, 530, 420, 135)
    local editModeDialog = RectFrame(250, 300, 500, 220)
    env.StaticPopup_ForEachShownDialog = function(callback)
      callback(env.StaticPopup1)
      callback(editModeDialog)
      assert.equals(0, frame.moves, "Do not reposition from inside Blizzard's iterator")
    end

    core.OnDialogUpdate(frame, 0.2)

    assert.equals(500, frame.x)
    assert.equals(290, frame.y)
    assert.equals(1, env.StaticPopup1.rectReads)
    assert.equals(1, editModeDialog.rectReads)
  end)

  it("returns to the normal position when the shared popup list becomes empty", function()
    local shownDialogs = { RectFrame(250, 300, 500, 220) }
    env.StaticPopup_ForEachShownDialog = function(callback)
      for _, popup in ipairs(shownDialogs) do callback(popup) end
    end
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(290, frame.y)

    shownDialogs = {}
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(500, frame.x)
    assert.equals(665, frame.y)
  end)

  it("discards partial bounds if an enumerated dialog cannot be measured", function()
    local normal = RectFrame(290, 530, 420, 135)
    local special = RectFrame(nil, nil, 500, 220)
    env.StaticPopup_ForEachShownDialog = function(callback)
      callback(normal)
      callback(special)
    end

    core.OnDialogUpdate(frame, 0.2)

    assert.equals(1, normal.rectReads)
    assert.equals(1, special.rectReads)
    assert.equals(0, frame.moves)
  end)

  it("converts different popup, dialog, and UIParent scales and origins", function()
    env.UIParent = RectFrame(100, 50, 1000, 800, 0.8)
    env.StaticPopup1 = RectFrame(300, 300, 200, 100, 1.2)
    frame.scale = 0.4
    core.OnDialogUpdate(frame, 0.2)
    assert.near(1000, frame.x, 0.0001)
    assert.near(790, frame.y, 0.0001)
  end)

  it("uses the space above when the dialog cannot fit below", function()
    env.StaticPopup1 = RectFrame(290, 100, 420, 100)
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(500, frame.x)
    assert.equals(350, frame.y)
  end)

  it("uses the right side when a tall stack leaves no vertical space", function()
    env.StaticPopup1 = RectFrame(0, 5, 300, 790)
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(520, frame.x)
    assert.equals(665, frame.y)
  end)

  it("uses the left side when the right side is also occupied", function()
    env.StaticPopup1 = RectFrame(700, 5, 300, 790)
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(480, frame.x)
    assert.equals(665, frame.y)
  end)

  it("keeps the choices at their normal position when no free side fits", function()
    env.StaticPopup1 = RectFrame(0, 0, 1000, 800)
    core.OnDialogUpdate(frame, 0.2)
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(500, frame.x)
    assert.equals(665, frame.y)
    assert.equals(1, frame.moves)
  end)

  it("preserves placement when a visible popup has secret coordinates", function()
    core.OnDialogUpdate(frame, 0.2)
    env.issecretvalue = function(value) return value == 123456 end
    env.StaticPopup1 = RectFrame(290, 123456, 420, 135)
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(1, frame.moves)
    assert.equals(665, frame.y)
  end)

  it("does not branch on secret popup visibility", function()
    local secret = {}
    env.issecretvalue = function(value) return value == secret end
    env.StaticPopup1 = RectFrame(290, 530, 420, 135)
    env.StaticPopup1.visible = secret
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(0, env.StaticPopup1.rectReads)
    assert.equals(0, frame.moves)
  end)

  it("waits for valid geometry instead of treating an unplaced popup as absent", function()
    env.StaticPopup1 = RectFrame(nil, nil, 420, 135)
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(0, frame.moves)

    env.StaticPopup1.left, env.StaticPopup1.bottom = 290, 530
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(520, frame.y)
  end)

  it("does not inspect forbidden popup frames", function()
    env.StaticPopup1 = RectFrame(290, 530, 420, 135)
    env.StaticPopup1.forbidden = true
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(0, env.StaticPopup1.rectReads)
    assert.equals(0, frame.moves)
  end)

  it("does not defer placement just because combat lockdown is active", function()
    env.InCombatLockdown = function() return true end
    env.StaticPopup1 = RectFrame(290, 530, 420, 135)
    core.OnDialogUpdate(frame, 0.2)
    assert.equals(520, frame.y)
  end)
end)
