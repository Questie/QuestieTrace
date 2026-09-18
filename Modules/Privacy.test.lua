---@param env table<string, any>
---@return QuestieTraceCore
local function LoadPrivacyModule(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}

  -- Mock WoW API functions used by Privacy.lua
  env.UnitName = function() return nil end
  env.IsInGroup = function() return false end
  env.IsInRaid = function() return false end

  -- Load globals.lua first (provides CopyPacked, etc.)
  local chunkGlobals = assert(loadfile("Modules/globals.lua"))
  setfenv(chunkGlobals, env)
  chunkGlobals()

  -- Then load Privacy.lua
  local chunk = assert(loadfile("Modules/Privacy.lua"))
  setfenv(chunk, env)
  chunk()

  return env.QuestieTraceCore
end

describe("Privacy", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadPrivacyModule(env)
  end)

  describe("ParseGUIDKind", function()
    it("should classify Player- prefix as player", function()
      assert.are.equal("player", Core.ParseGUIDKind("Player-4618-0053656F"))
    end)

    it("should classify Creature- prefix as npc", function()
      assert.are.equal("npc", Core.ParseGUIDKind("Creature-0-6783-1-269-2980-00002C50D7"))
    end)

    it("should classify Pet- prefix as npc", function()
      assert.are.equal("npc", Core.ParseGUIDKind("Pet-0-6783-1-269-2980-00002C50D7"))
    end)

    it("should classify Vehicle- prefix as npc", function()
      assert.are.equal("npc", Core.ParseGUIDKind("Vehicle-0-6783-1-269-2980-00002C50D7"))
    end)

    it("should classify GameObject- prefix as object", function()
      assert.are.equal("object", Core.ParseGUIDKind("GameObject-0-6783-1-269-2980-00002C50D7"))
    end)

    it("should classify Item- prefix as item", function()
      assert.are.equal("item", Core.ParseGUIDKind("Item-4-0-000012"))
    end)

    it("should return nil for unrecognized prefix", function()
      assert.is_nil(Core.ParseGUIDKind("Corpse-0-6783-1-269-2980-00002C50D7"))
      assert.is_nil(Core.ParseGUIDKind("DynamicObject-0-6783-1-269-2980-00002C50D7"))
      assert.is_nil(Core.ParseGUIDKind("Nonsense-guid"))
    end)

    it("should return nil for nil input", function()
      assert.is_nil(Core.ParseGUIDKind(nil))
    end)

    it("should return nil for non-string input", function()
      assert.is_nil(Core.ParseGUIDKind(123))
    end)
  end)

  describe("IsPlayerGUID", function()
    it("should return true for Player- GUID", function()
      assert.is_true(Core.IsPlayerGUID("Player-4618-0053656F"))
    end)

    it("should return false for Creature- GUID", function()
      assert.is_false(Core.IsPlayerGUID("Creature-0-6783-1-269-2980-00002C50D7"))
    end)

    it("should return false for unrecognized GUID", function()
      assert.is_false(Core.IsPlayerGUID("Corpse-0-6783-1-269-2980-00002C50D7"))
    end)

    it("should return false for nil", function()
      assert.is_false(Core.IsPlayerGUID(nil))
    end)
  end)

describe("GetPrivacyNameSet", function()
    it("should include local player name and split into parts", function()
      env.UnitName = function(token)
        if token == "player" then return "John Doe" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local names = Core.GetPrivacyNameSet()
      assert.is_true(names["John Doe"])
      assert.is_true(names["John"])
      assert.is_true(names["Doe"])
    end)

    it("should include local player when UnitName returns firstname, lastname separately (future-proof)", function()
      env.UnitName = function(token)
        if token == "player" then return "John", "Doe" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local names = Core.GetPrivacyNameSet()
      assert.is_true(names["John"])
      assert.is_true(names["Doe"])
    end)

    it("should include party members with both firstname and lastname", function()
      env.UnitName = function(token)
        if token == "player" then return "Player1" end
        if token == "party1" then return "PartyMember", "One" end
        if token == "party2" then return "PartyMember", "Two" end
        return nil
      end
      env.IsInGroup = function() return true end
      env.IsInRaid = function() return false end

      local names = Core.GetPrivacyNameSet()
      assert.is_true(names["Player1"])
      assert.is_true(names["PartyMember"])
      assert.is_true(names["One"])
      assert.is_true(names["Two"])
    end)

    it("should include raid members with both firstname and lastname", function()
      env.UnitName = function(token)
        if token == "player" then return "Player1" end
        if token == "raid1" then return "Raider", "One" end
        if token == "raid2" then return "Raider", "Two" end
      end
      env.IsInGroup = function() return true end
      env.IsInRaid = function() return true end

      local names = Core.GetPrivacyNameSet()
      assert.is_true(names["Player1"])
      assert.is_true(names["Raider"])
      assert.is_true(names["One"])
      assert.is_true(names["Two"])
    end)

    it("should handle nil UnitName gracefully", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local names = Core.GetPrivacyNameSet()
      assert.are.same({}, names)
    end)

    it("should not include empty names", function()
      env.UnitName = function() return "" end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local names = Core.GetPrivacyNameSet()
      assert.is_nil(names[""])
    end)

    it("should capture all return values from UnitName for local player", function()
      -- Tests the GetUnitNameParts wrapper behavior through GetPrivacyNameSet
      env.UnitName = function(token)
        if token == "player" then return "A", "B", "C" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local names = Core.GetPrivacyNameSet()
      assert.is_true(names["A"])
      assert.is_true(names["B"])
      -- third return value (if any) is ignored per wrapper design
    end)

    it("should split first return value by spaces", function()
      env.UnitName = function(token)
        if token == "player" then return "First Middle Last" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local names = Core.GetPrivacyNameSet()
      assert.is_true(names["First Middle Last"])
      assert.is_true(names["First"])
      assert.is_true(names["Middle"])
      assert.is_true(names["Last"])
    end)

    it("should include party members when UnitName returns two values", function()
      env.UnitName = function(token)
        if token == "player" then return "Player" end
        if token == "party1" then return "First", "Last" end
        return nil
      end
      env.IsInGroup = function() return true end
      env.IsInRaid = function() return false end

      local names = Core.GetPrivacyNameSet()
      assert.is_true(names["Player"])
      assert.is_true(names["First"])
      assert.is_true(names["Last"])
    end)
  end)

  describe("SanitizeText", function()
    it("should redact local player full name", function()
      env.UnitName = function(token)
        if token == "player" then return "John Doe" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local result = Core.SanitizeText("Greetings, John Doe, welcome!")
      assert.are.equal("Greetings, <name>, welcome!", result)
    end)

    it("should redact local player firstname only", function()
      env.UnitName = function(token)
        if token == "player" then return "John Doe" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local result = Core.SanitizeText("Hello John, how are you?")
      assert.are.equal("Hello <name>, how are you?", result)
    end)

    it("should redact local player lastname only", function()
      env.UnitName = function(token)
        if token == "player" then return "John Doe" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local result = Core.SanitizeText("Goodbye Doe")
      assert.are.equal("Goodbye <name>", result)
    end)

    it("should redact party member firstname and lastname", function()
      env.UnitName = function(token)
        if token == "player" then return "Player1" end
        if token == "party1" then return "PartyMember", "One" end
        return nil
      end
      env.IsInGroup = function() return true end
      env.IsInRaid = function() return false end

      local result = Core.SanitizeText("PartyMember and One are here.")
      assert.are.equal("<name> and <name> are here.", result)
    end)

    it("should handle names with pattern-magic characters (apostrophe, hyphen)", function()
      env.UnitName = function(token)
        if token == "player" then return "Bri'ka-Nn" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local result = Core.SanitizeText("Hello Bri'ka-Nn!")
      assert.are.equal("Hello <name>!", result)
    end)

    it("should return unchanged text when no names found", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local text = "Your reputation with Timbermaw Hold increased."
      assert.are.equal(text, Core.SanitizeText(text))
    end)

    it("should return nil unchanged", function()
      assert.is_nil(Core.SanitizeText(nil))
    end)

    it("should return empty string unchanged", function()
      assert.are.equal("", Core.SanitizeText(""))
    end)

    it("should handle multiple occurrences of same name", function()
      env.UnitName = function(token)
        if token == "player" then return "Ann" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local result = Core.SanitizeText("Ann and Ann and Ann")
      assert.are.equal("<name> and <name> and <name>", result)
    end)

    it("should handle substring matches correctly (greedy)", function()
      env.UnitName = function(token)
        if token == "player" then return "Ann" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      -- Note: substring matching is intentional behavior
      local result = Core.SanitizeText("Announce Ann")
      assert.are.equal("<name>ounce <name>", result)
    end)

    it("should redact longer overlapping names before shorter names", function()
      env.UnitName = function(token)
        if token == "player" then return "Bobby" end
        if token == "party1" then return "Bob" end
        return nil
      end
      env.IsInGroup = function() return true end
      env.IsInRaid = function() return false end

      local result = Core.SanitizeText("Bobby and Bob are here.")
      assert.are.equal("<name> and <name> are here.", result)
    end)
  end)

  describe("SanitizeChatMsgArgs", function()
    it("should strip playerName (arg 2) and playerName2 (arg 5)", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local args = Core.PackArgs(
        "Grimtotem receives loot: [Item].", "Grimtotem", "", "",
        "", "", 0, 0, "", 0, 1, "Player-4618-0053656F", 0, false, false, false, false
      )

      local sanitized = Core.SanitizeChatMsgArgs(args)
      assert.is_nil(sanitized[2])
      assert.is_nil(sanitized[5])
    end)

    it("should redact playerName/playerName2 from text argument", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local args = Core.PackArgs(
        "Grimtotem receives loot.", "Grimtotem", "", "", "", "", 0, 0, "", 0, 1,
        "Creature-0-6783-1-269-2980-00002C50D7", 0, false, false, false, false
      )

      local sanitized = Core.SanitizeChatMsgArgs(args)
      assert.are.equal("<name> receives loot.", sanitized[1])
    end)

    it("should apply SanitizeText as second pass for local player/roster", function()
      env.UnitName = function(token)
        if token == "player" then return "LocalPlayer" end
        return nil
      end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local args = Core.PackArgs(
        "Grimtotem and LocalPlayer did something.", "Grimtotem", "", "", "", "", 0, 0, "", 0, 1,
        "Creature-0-6783-1-269-2980-00002C50D7", 0, false, false, false, false
      )

      local sanitized = Core.SanitizeChatMsgArgs(args)
      assert.are.equal("<name> and <name> did something.", sanitized[1])
    end)

    it("should strip player GUID (arg 12)", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local args = Core.PackArgs(
        "text", "player", "", "", "", "", 0, 0, "", 0, 1,
        "Player-4618-0053656F", 0, false, false, false, false
      )

      local sanitized = Core.SanitizeChatMsgArgs(args)
      assert.is_nil(sanitized[12])
    end)

    it("should preserve non-player GUID (arg 12)", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local args = Core.PackArgs(
        "text", "player", "", "", "", "", 0, 0, "", 0, 1,
        "Creature-0-6783-1-269-2980-00002C50D7", 0, false, false, false, false
      )

      local sanitized = Core.SanitizeChatMsgArgs(args)
      assert.are.equal("Creature-0-6783-1-269-2980-00002C50D7", sanitized[12])
    end)

    it("should discard unrecognized GUID kinds (arg 12)", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local args = Core.PackArgs(
        "text", "player", "", "", "", "", 0, 0, "", 0, 1,
        "Corpse-0-6783-1-269-2980-00002C50D7", 0, false, false, false, false
      )

      local sanitized = Core.SanitizeChatMsgArgs(args)
      assert.is_nil(sanitized[12])
    end)

    it("should not mutate the original args", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local args = Core.PackArgs(
        "text", "player", "", "", "", "", 0, 0, "", 0, 1,
        "Player-4618-0053656F", 0, false, false, false, false
      )

      Core.SanitizeChatMsgArgs(args)
      assert.are.equal("player", args[2])
      assert.are.equal("Player-4618-0053656F", args[12])
    end)

    it("should preserve other args (languageName, channelName, specialFlags, etc.)", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local args = Core.PackArgs(
        "text", "player", "Common", "1. Trade", "target", "GM", 1, 2,
        "Trade", 1, 42, "Creature-0-6783-1-269-2980-00002C50D7",
        12345, false, false, false, false
      )

      local sanitized = Core.SanitizeChatMsgArgs(args)
      assert.are.equal("Common", sanitized[3])
      assert.are.equal("1. Trade", sanitized[4])
      assert.are.equal("GM", sanitized[6])
      assert.are.equal(1, sanitized[7])
      assert.are.equal(2, sanitized[8])
      assert.are.equal("Trade", sanitized[9])
      assert.are.equal(1, sanitized[10])
      assert.are.equal(42, sanitized[11])
      assert.are.equal(12345, sanitized[13]) -- bnSenderID
    end)

    it("should return empty args for empty/nil text", function()
      env.UnitName = function() return nil end
      env.IsInGroup = function() return false end
      env.IsInRaid = function() return false end

      local args = Core.PackArgs(
        "", "player", "", "", "", "", 0, 0, "", 0, 1,
        "Player-4618-0053656F", 0, false, false, false, false
      )

      local sanitized = Core.SanitizeChatMsgArgs(args)
      assert.are.equal("", sanitized[1])
    end)

    it("should handle nil args gracefully", function()
      local sanitized = Core.SanitizeChatMsgArgs(nil)
      assert.is_table(sanitized)
      assert.are.equal(0, sanitized.n)
    end)
  end)
end)