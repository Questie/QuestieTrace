# AGENTS.md - QuestieTrace WoW Addon

QuestieTrace is a World of Warcraft Classic addon written in Lua 5.1 that provides event tracing and function tracing capabilities for debugging and development.

## Privacy (CRITICAL)

User privacy is of the utmost importance. QuestieTrace must **never** collect or persist:

- Player character names (including the local player's own name), or realm-qualified names
- Guild names
- Player GUIDs (e.g. `Player-1234-0053656F`) — only NPC/Creature/GameObject/Vehicle/Item GUIDs are allowed
- Any free-form text (chat messages, gossip/quest text) that embeds a player name

Rules for any code that touches trackers, dumps, or raw event recording:

- Any GUID captured from a WoW API (`UnitGUID`, `GetLootSourceInfo`, chat message `guid` args, etc.) **must**
  be classified with `Core.ParseGUIDKind` (see `Modules/globals.lua`) before being stored. Discard the value
  entirely if the kind is `"player"` or unrecognized.
- Any free text captured from a WoW API (`C_GossipInfo.GetText`, `GetQuestText`, chat message `text`, etc.)
  **must** be passed through `Core.SanitizeText` before being stored.
- Chat message events (`CHAT_MSG_*`) **must** go through `Core.SanitizeChatMsgArgs` before being recorded to
  `session.events` or dispatched to trackers.
- Never introduce a tracker that stores `UnitName(...)`/`UnitGUID(...)` for a token that can resolve to another
  player without filtering through the helpers above.
- Do not rely on `issecretvalue()` or similar "sometimes correct" global flags as the sole privacy guard —
  privacy filtering must use explicit, deterministic, manually-reviewed checks (e.g. GUID prefix matching,
  known-name substitution) that we own and can test.
- Any new tracker or dump provider must be reviewed against this policy, and covered by a test that proves
  player names/GUIDs are filtered out (see `Tests/run.lua`).

## Build & Test Commands

### Prerequisites

- Lua 5.1, luarocks, luacheck
- Luarocks packages: `bit32`, `busted`, `luafilesystem`

### Tests

```bash
# Run busted-style unit tests
busted -p ".test.lua" .

# Run custom test runner (main test suite)
lua Tests/run.lua
```

### Linting (Luacheck)

```bash
luacheck -q .
```

### Language Server

```bash
lua-language-server --check=.
```

> **Note:** Language server checks take ~3 minutes due to Blizzard UI documentation. Use 180s timeout.

## Project Structure

```
QuestieTrace.lua          - Main addon entry point (initialization, event handling)
QuestieTrace_UI.lua       - User interface code
Modules/                  - Core modules and tracking systems
    globals.lua           - Global variables, constants, shared state
    Trackers/             - Individual tracking modules
        PlayerIdentity.lua     - Player identity tracking
        UnitLevel.lua          - Unit level tracking
        Position.lua           - Position/coordinate tracking
        Loot.lua               - Loot tracking
        Reputation.lua         - Reputation tracking
        QuestLog.lua           - Quest log tracking
        CompletedQuests.lua    - Completed quests tracking
        QuestDialog.lua        - Quest dialog/gossip tracking
        UnitInteraction.lua    - Unit interaction tracking
        UnitState.lua          - Unit state (level/classification/reaction) tracking
        GroupState.lua         - Group/raid state tracking
        SkillLines.lua         - Skill lines tracking
        SpellBook.lua          - Spell book tracking
        ResetTime.lua          - Daily/weekly reset tracking
    Dumps/                - Data dump providers
        MapHierarchy.lua   - C_Map hierarchy data dump
    Export/               - Data export functionality
        Encoding.lua       - Codec support checks + CBOR/compression encoding
        Export.lua         - Export utilities
        ExportUI.lua       - Export window UI
    Localization/         - Translation system
        l10n.lua           - Translation lookup/locale resolution (Core.l10n)
        Translations/      - One file per feature area's translatable strings
            ExportUI.lua   - Strings used by Export/ExportUI.lua and Export.lua
specs/                    - Design specifications
    ARCHITECTURE_SPEC.md  - Overall architecture
    TRACKER_SPEC.md       - Tracker interface specification
    EVENT_CATALOG.md      - Event catalog
    SCHEMA_SPEC.md        - Data schema specification
    TRACE_FILE_SPEC.md    - Trace file format
    UI_SPEC.md            - UI specification
    FUNCTION_EMULATION_SPEC.md - Function emulation
    LuaLS_annotations.md  - LuaLS type annotation guide
    README.md             - Specs index
tasks/                    - Implementation task documents
Documentation/
    WoW-API/              - Blizzard API documentation (Functions-AI, Events)
    WoW-Event/            - Blizzard event documentation
Tests/
    run.lua               - Test runner with mocked WoW API
    Empty.lua             - Empty test file
tools/
    trace-analyzer/       - TypeScript/React trace analysis tool
Traces/                   - Example trace files
Libs/                     - Third-party libraries
```

## Code Style

### Module System

QuestieTrace uses a simple module pattern. Trackers register themselves with the core.

**Creating a tracker** (in `Modules/Trackers/`):

```lua
---@class MyTracker
local MyTracker = {}
MyTracker.__index = MyTracker

function MyTracker:New()
    local self = setmetatable({}, MyTracker)
    self:Init()
    return self
end

function MyTracker:Init()
    -- Register events, initialize state
end

function MyTracker:OnEvent(event, ...)
    -- Handle events
end

-- Register with core
QuestieTraceCore:RegisterTracker("MyTracker", MyTracker)
```

### Standard File Boilerplate

```lua
---@class MyTracker
local MyTracker = {}
MyTracker.__index = MyTracker

-- Performance: alias frequently used functions
local tinsert = table.insert
local band = bit.band
```

### Formatting

- Indent: 2 spaces (configured in .luarc.json)
- Line endings: LF
- Quote style: double quotes
- Max line length: 160 (formatter) / 140 (luacheck)
- No trailing whitespace

### Type Annotations (LuaCATS / EmmyLua)

Use annotations compatible with the sumneko.lua language server:

```lua
---@class ClassName
---@field fieldName type
---@param paramName type @Description
---@return type @Description
```

Reference: `./specs/LuaLS_annotations.md`

### Naming Conventions

| Category             | Convention         | Example                          |
|----------------------|--------------------|----------------------------------|
| Tracker names        | PascalCase         | `QuestLogTracker`, `LootTracker` |
| Local variables      | camelCase          | `playerName`, `currentTime`      |
| Local functions      | `_` + PascalCase   | `_HelperFunction`                |
| Tracker methods      | PascalCase         | `Tracker:OnEvent()`              |
| Private tables       | `_` + PascalCase   | `_QuestLogTracker`               |
| Constants            | UPPER_SNAKE_CASE   | `MAX_SESSIONS`, `SCHEMA_VERSION` |
| File names (trackers)| PascalCase.lua     | `QuestLog.lua`, `Loot.lua`       |
| Test files           | Source + .test.lua | (Not used - tests in Tests/)     |

### Error Handling

- `print("ERROR: ...")` - For critical errors (no structured logging yet)
- `pcall` for risky operations
- `error()` for hard input validation failures

### Private vs Public Members

- Public: directly on the module table (`MyTracker.field`, `function MyTracker:Method()`)
- Private: file-local `local function helper()` for truly internal code

### Expansion-Specific Code

QuestieTrace targets Classic Era (1.14.3). Use feature detection:

```lua
if C_GossipInfo then
    -- Retail/Wrath+ API available
else
    -- Classic API
end
```

### Functions vs Methods

Prefer plain **functions** over **methods** when `self` is not needed. This avoids unnecessary method dispatch overhead and makes the code simpler to test and mock.

**Bad** — unnecessary method syntax:
```lua
function MyTracker:HelperFunction()
    -- self is not used
    return someCalculation()
end
```

**Good** — plain function when `self` is unused:
```lua
function MyTracker.HelperFunction()
    return someCalculation()
end
```

**Good** — method when `self` is actually used:
```lua
function MyTracker:OnEvent(event, ...)
    self:ProcessEvent(event, ...)
end
```

## Test Conventions

### Custom Test Runner (Primary)

Tests live in `Tests/run.lua` and use isolated Lua environments with mocked WoW APIs.

```lua
---@param trackerFiles string[]
---@return TestRuntime
local function NewRuntime(trackerFiles)
    -- Creates isolated environment with mocked globals
    -- Loads globals.lua, tracker files, QuestieTrace.lua
    -- Returns runtime with core, timers, frame mocks
end

---@param runtime TestRuntime
---@param event string
local function SendEvent(runtime, event)
    runtime.frame.OnEvent(runtime.frame, event)
end

---@param runtime TestRuntime
---@param target number
local function AdvanceTo(runtime, target)
    -- Advances virtual time, executes due timers
end
```

#### Test Structure

```lua
local function TestFeatureName()
    local runtime = NewRuntime({ "Trackers/SpecificTracker.lua" })
    -- Setup mocks
    runtime.core.StartCapture("test name")
    -- Send events, advance time
    -- Assert results
end

local tests = {
    { name = "description", run = TestFeatureName },
}

for _, test in ipairs(tests) do
    local ok, err = pcall(test.run)
    print((ok and "PASS " or "FAIL ") .. test.name)
    if not ok then print(err) end
end
```

#### Mocking Guidelines

- Override `_G.*` globals in `runtime.env`
- Use `runtime.env.FunctionName = function() ... end`
- Mock WoW API functions needed by the tracker under test
- Use `AdvanceTo()` to test timer-based behavior

#### Assertions

- Use `assert(condition, message)` for all assertions
- Compare tables with custom comparison if needed
- Check timer behavior with `AdvanceTo` and call counts

### Busted-Style Tests (Secondary)

For new tests that fit the busted framework pattern, place `*.test.lua` files alongside the source code they test:

```lua
-- Modules/Trackers/MyTracker.test.lua
describe("MyTracker", function()
    it("should handle EVENT_NAME correctly", function()
        -- test implementation
    end)
end)
```

Run with: `busted -p ".test.lua" .`

## CI Pipeline

CI runs on every push/PR: luacheck lint, custom test runner (`lua Tests/run.lua`), and busted tests (`busted -p ".test.lua" .`). Configured in `.github/workflows/ci.yml`.

## Test Requirements

Any change to a tracker **must** include corresponding test additions or adjustments:

- Adding a new tracker → add test cases for its event handling in `Tests/run.lua` or create a `Modules/Trackers/NewTracker.test.lua`
- Changing event handling behavior → update affected tests
- Adding new public API → add tests for it

Run the full suite before considering a change done:

```bash
lua Tests/run.lua
busted -p ".test.lua" .
```

## Tracker Development Guidelines

### Adding a New Tracker

1. Create `Modules/Trackers/NewTracker.lua` following the module pattern
2. Add to `QuestieTrace-Classic.toc` (and other expansion TOCs) in load order under `Modules/Trackers/`
3. Register with `QuestieTraceCore:RegisterTracker("NewTracker", NewTracker)`
4. Add test cases in `Tests/run.lua`
5. Update relevant specs in `specs/`

### Event Registration

```lua
function MyTracker:Init()
    self.frame = CreateFrame("Frame")
    self.frame:RegisterEvent("EVENT_NAME")
    self.frame:SetScript("OnEvent", function(_, event, ...)
        self:OnEvent(event, ...)
    end)
end
```

### Timer Usage

```lua
C_Timer.After(delay, function()
    -- Delayed work
end)
```

### Data Storage

Trackers store data in the session via `QuestieTraceCore`:

```lua
local session = QuestieTraceCore:GetCurrentSession()
session.events[#session.events + 1] = { t = time, e = "EVENT_NAME", data = {...} }
```

## Localization

User-facing strings (UI text, error messages shown to the user) go through `Core.l10n` (`Modules/Localization/l10n.lua`). Internal/debug-only strings do not need translation.

**Using a translated string** (in any module, after `l10n.lua` has loaded):

```lua
---@type l10n
local l10n = Core.l10n

myFontString:SetText(l10n("Close"))
```

**Adding a new translatable string:**

1. Add the English string as a key to the relevant file under `Modules/Localization/Translations/` (one file per feature area, e.g. `ExportUI.lua` for `Export/ExportUI.lua` + `Export/Export.lua` strings). Create a new file if none fits.
2. Provide an entry for every supported locale: `enUS`, `deDE`, `esES`, `esMX`, `frFR`, `koKR`, `ptBR`, `ruRU`, `zhCN`, `zhTW`. Use `["enUS"] = true` (the key itself is the enUS string). Mark any AI-generated value with a trailing `-- 🤖` comment (see below).
3. Add the new translation file to all `*.toc` files, after `Modules/Localization/l10n.lua` and before any module that calls `l10n(...)` with those keys.
4. Call `l10n("Your English string")` wherever the string is displayed. `string.format`-style `%s`/`%d` placeholders are supported via extra args: `l10n("Loaded %d sessions", count)`.

Missing translations automatically fall back to the enUS key (used as a format string), so partial translations never crash.

### AI-Generated Translations (REQUIRED)

Any translation value produced by an AI/LLM (rather than written or verified by a fluent human) **must** be
marked with a trailing `-- 🤖` comment on that line:

```lua
["Close"] = {
  ["enUS"] = true,
  ["deDE"] = "Schließen", -- 🤖
  ["frFR"] = "Fermer",    -- 🤖
},
```

Rules:

- One marker per locale line, placed after the trailing comma: `["deDE"] = "…", -- 🤖`.
- Never mark the `["enUS"] = true` entry — the key itself is the English source, not a translation.
- Remove the marker only when a fluent speaker has reviewed and confirmed (or corrected) that value.
- All existing translations under `Modules/Localization/Translations/` are already marked; when adding a new
  locale entry or regenerating an existing one with AI, add the marker in the same commit.
- The marker is a Lua comment, so it never affects the string value at runtime.

## Specifications

All design decisions are documented in `specs/`. Before implementing significant changes:

1. Check relevant spec files
2. Update specs if architecture changes
3. Key specs:
   - `ARCHITECTURE_SPEC.md` - Overall system design
   - `TRACKER_SPEC.md` - Tracker interface contract
   - `SCHEMA_SPEC.md` - Session data schema
   - `EVENT_CATALOG.md` - Supported events

## WoW API Documentation

- Primary: https://warcraft.wiki.gg
- Local: `./Documentation/WoW-API/Functions-AI/` and `./Documentation/WoW-Event/`

## Forbidden Patterns

- Never use code from `Trace/` (old unused UI)
- Never use code from `.shit/` (manual trash)
- Don't add test files to TOC
- Don't commit trace files or dumps to repo
