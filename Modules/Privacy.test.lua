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

  -- Mock the Blizzard global strings Core.IsAllowedSystemMessage reads from
  -- _G (normally populated by Interface/GlobalStrings.lua on the client).
  -- Wording taken from Documentation/GlobalStrings.1.60.1.69913.csv (enUS).
  env.ERR_QUEST_ACCEPTED_S = "Quest accepted: %s"
  env.ERR_QUEST_COMPLETE_S = "%s completed."
  env.ERR_QUEST_FAILED_S = "%s failed."
  env.ERR_QUEST_FAILED_BAG_FULL_S = "%s failed: Inventory is full."
  env.ERR_QUEST_FAILED_WRONG_RACE = "That quest is not available to your race."
  env.ERR_QUEST_REWARD_EXP_I = "Experience gained: %d."
  env.ERR_QUEST_REWARD_MONEY_S = "Received %s."
  env.ERR_QUEST_LOG_FULL = "Your quest log is full."
  env.ERR_QUEST_FORCE_REMOVED_S = "The quest %s has been removed from your quest log."
  env.ERR_QUEST_ALREADY_DONE = "You have completed that quest."
  env.ERR_QUEST_ALREADY_DONE_DAILY = "You have completed that daily quest today."
  env.ERR_QUEST_ALREADY_ON = "You are already on that quest."
  env.ERR_ZONE_EXPLORED_XP = "Discovered %s: %d experience gained"
  env.ERR_SKILL_GAINED_S = "You have gained the %s skill."
  env.ERR_SKILL_UP_SI = "Your skill in %s has increased to %d."
  env.ERR_LEARN_ABILITY_S = "You have learned a new ability: %s."
  env.ERR_LEARN_SPELL_S = "You have learned a new spell: %s."
  env.ERR_LEARN_RECIPE_S = "You have learned how to create a new item: %s."
  env.LEVEL_REQUIRED = "Req level %d"

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

  describe("IsAllowedSystemMessage", function()
    it("should allow a quest accepted message", function()
      assert.is_true(Core.IsAllowedSystemMessage("Quest accepted: A Humble Task"))
    end)

    it("should allow a quest completed message", function()
      assert.is_true(Core.IsAllowedSystemMessage("Grace of An'she and Mu'sha completed."))
    end)

    it("should allow a quest failed message", function()
      assert.is_true(Core.IsAllowedSystemMessage("The Hunt Begins failed."))
    end)

    it("should allow an experience gained message", function()
      assert.is_true(Core.IsAllowedSystemMessage("Experience gained: 170."))
    end)

    it("should allow a money received message", function()
      assert.is_true(Core.IsAllowedSystemMessage("Received 17 Copper."))
    end)

    it("should allow a skill gained message", function()
      assert.is_true(Core.IsAllowedSystemMessage("You have gained the Herbalism skill."))
    end)

    it("should allow a skill increased message", function()
      assert.is_true(Core.IsAllowedSystemMessage("Your skill in Herbalism has increased to 50."))
    end)

    it("should allow a learned ability message", function()
      assert.is_true(Core.IsAllowedSystemMessage("You have learned a new ability: |cff71d5ff|Hspell:2383:0|h[Find Herbs]|h|r."))
    end)

    it("should allow a learned spell message", function()
      assert.is_true(Core.IsAllowedSystemMessage("You have learned a new spell: |cff71d5ff|Hspell:1126:0|h[Mark of the Wild]|h|r."))
    end)

    it("should allow a learned recipe message", function()
      assert.is_true(Core.IsAllowedSystemMessage("You have learned how to create a new item: Bronze Tube."))
    end)

    it("should allow a static quest log full message", function()
      assert.is_true(Core.IsAllowedSystemMessage("Your quest log is full."))
    end)

    it("should allow a static wrong-race quest failure message", function()
      assert.is_true(Core.IsAllowedSystemMessage("That quest is not available to your race."))
    end)

    it("should allow a level requirement message", function()
      assert.is_true(Core.IsAllowedSystemMessage("Req level 10"))
    end)

    it("should reject a guild join message", function()
      assert.is_false(Core.IsAllowedSystemMessage("Tommy Naid has joined the guild."))
    end)

    it("should reject a guild leave message", function()
      assert.is_false(Core.IsAllowedSystemMessage("Arrow Beth has left the guild."))
    end)

    it("should reject an offline notification", function()
      assert.is_false(Core.IsAllowedSystemMessage("Pinky Rakiyash has gone offline."))
    end)

    it("should reject an online notification with a player hyperlink", function()
      assert.is_false(Core.IsAllowedSystemMessage("|Hplayer:Red Axl|h[Red Axl]|h has come online."))
    end)

    it("should reject a guild invite message", function()
      assert.is_false(Core.IsAllowedSystemMessage("|Hplayer:Red Axl|h[Red Axl]|h invites you to join Ony Fans."))
    end)

    it("should reject a guild promote message", function()
      assert.is_false(Core.IsAllowedSystemMessage("Red Axl has promoted Ar Droll to Common."))
    end)

    it("should reject an unrelated server broadcast", function()
      assert.is_false(Core.IsAllowedSystemMessage("[SERVER] We're restarting soon."))
    end)

    it("should reject a rested-state message not on the allowlist", function()
      assert.is_false(Core.IsAllowedSystemMessage("You are no longer rested."))
    end)

    it("should reject non-string input", function()
      assert.is_false(Core.IsAllowedSystemMessage(nil))
      assert.is_false(Core.IsAllowedSystemMessage(42))
    end)

    it("should reject empty string input", function()
      assert.is_false(Core.IsAllowedSystemMessage(""))
    end)
  end)
end)
