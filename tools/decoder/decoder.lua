#!/usr/bin/env lua
-- Decodes QuestieTrace export string(s) (as produced by Core.BuildExportString)
-- back into Lua tables and prints/saves them.
--
-- Reverses the pipeline in Modules/Export/Encoding.lua:
--   EncodeForPrint -> Deflate decompress -> CBOR deserialize
--
-- Usage:
--   lua decoder.lua                       -- batch: decode every .txt file in
--                                                  ./input/ into ./output/
--   lua decoder.lua <trace-file>           -- decode a single .txt file to stdout
--   lua decoder.lua <trace-file> -o <path> -- decode a single .txt file to <path>
--   lua decoder.lua --help
--
-- See README.md in this directory for details.

local lfs = require("lfs")

local scriptDir = arg[0]:match("^(.*)/[^/]+$") or "."
local defaultInputDir = scriptDir .. "/input"
local defaultOutputDir = scriptDir .. "/output"

local LibDeflate = dofile(scriptDir .. "/dependencies/LibDeflate.lua")
local BlizzardCBOR = dofile(scriptDir .. "/dependencies/BlizzardCBOR.lua")

---@param message string
local function Fail(message)
  io.stderr:write("error: " .. message .. "\n")
  os.exit(1)
end

---@param out file*
local function PrintUsage(out)
  out:write([[
Usage: lua decoder.lua [options] [trace-file]

Decodes QuestieTrace export string(s) into Lua table(s).

With no arguments, runs in batch mode: every .txt file in ./input/ (relative
to this script) is decoded and written as ./output/<basename>.lua. Both
directories are created automatically if they don't exist yet.

Arguments:
  [trace-file]          Optional. Path to a single .txt file containing a
                         QuestieTrace export string. When given, only that
                         file is decoded instead of running batch mode.

Options:
  -o, --output <path>   Only valid together with [trace-file]. Write the
                         decoded Lua table to <path> instead of stdout.
                         <path> may also be a directory, in which case the
                         output is written as <path>/<basename>.lua.
  -h, --help            Show this help message.
]])
end

---@param args string[]
---@return string|nil inputPath
---@return string|nil outputPath
local function ParseArgs(args)
  local inputPath, outputPath
  local i = 1
  while i <= #args do
    local a = args[i]
    if a == "-h" or a == "--help" then
      PrintUsage(io.stdout)
      os.exit(0)
    elseif a == "-o" or a == "--output" then
      outputPath = args[i + 1]
      if not outputPath then
        Fail(a .. " requires a path argument")
      end
      i = i + 1
    elseif a:sub(1, 1) == "-" then
      Fail("unknown option '" .. a .. "'")
    elseif not inputPath then
      inputPath = a
    else
      Fail("unexpected extra argument '" .. a .. "'")
    end
    i = i + 1
  end

  if outputPath and not inputPath then
    Fail("-o/--output requires [trace-file] to also be given")
  end

  return inputPath, outputPath
end

---@param path string
---@return string|nil mode "file", "directory", or nil if it doesn't exist
local function PathMode(path)
  local attrs = lfs.attributes(path)
  return attrs and attrs.mode or nil
end

---@param path string
---@return boolean
local function HasTxtExtension(path)
  return path:match("%.txt$") ~= nil
end

---@param path string
local function EnsureDirectoryExists(path)
  if PathMode(path) == nil then
    local ok, success, mkdirErr = pcall(lfs.mkdir, path)
    if not ok then
      Fail("failed to create directory '" .. path .. "': " .. tostring(success))
    elseif not success then
      Fail("failed to create directory '" .. path .. "': " .. tostring(mkdirErr))
    end
  elseif PathMode(path) ~= "directory" then
    Fail("path '" .. path .. "' exists and is not a directory")
  end
end

---@param path string
---@return string|nil content
---@return string|nil err
local function ReadFile(path)
  local file, openErr = io.open(path, "r")
  if not file then
    return nil, "cannot open input file '" .. path .. "': " .. tostring(openErr)
  end
  local raw = file:read("*a")
  file:close()
  if not raw or raw:match("^%s*$") then
    return nil, "input file '" .. path .. "' is empty"
  end
  return raw, nil
end

---@param raw string
---@return string|nil encoded
---@return string|nil err
local function ExtractEncodedPayload(raw)
  local encoded = raw:match("^%s*!QuestieTrace:%d+!(.-)!End:QuestieTrace:%d+!%s*$")
  if not encoded then
    return nil, "export string must be wrapped in !QuestieTrace:N! and !End:QuestieTrace:N! markers"
  end
  if encoded == "" then
    return nil, "export markers found but payload is empty"
  end
  return encoded, nil
end

---@param encoded string
---@return table|nil payload
---@return string|nil err
local function DecodePayload(encoded)
  local compressed = LibDeflate:DecodeForPrint(encoded)
  if not compressed then
    return nil, "payload is not valid QuestieTrace export data (DecodeForPrint failed)"
  end

  local cbor = LibDeflate:DecompressDeflate(compressed)
  if not cbor then
    return nil, "payload could not be decompressed (DecompressDeflate failed)"
  end

  local ok, payload = pcall(BlizzardCBOR.DeserializeCBOR, cbor)
  if not ok then
    return nil, "payload could not be deserialized as CBOR: " .. tostring(payload)
  end

  if type(payload) ~= "table" then
    return nil, "decoded payload is not a table (got " .. type(payload) .. ")"
  end

  return payload, nil
end

---@param value any
---@param indent string
local function Serialize(value, indent)
  local t = type(value)
  if t == "table" then
    local nextIndent = indent .. "  "
    local parts = { "{\n" }
    for k, v in pairs(value) do
      local key
      if type(k) == "number" then
        key = "[" .. k .. "]"
      else
        key = "[" .. string.format("%q", tostring(k)) .. "]"
      end
      parts[#parts + 1] = nextIndent .. key .. " = " .. Serialize(v, nextIndent) .. ",\n"
    end
    parts[#parts + 1] = indent .. "}"
    return table.concat(parts)
  elseif t == "string" then
    return string.format("%q", value)
  else
    return tostring(value)
  end
end

--- Runs the full decode pipeline for a single input file.
---@param inputPath string
---@return string|nil serialized
---@return string|nil err
local function DecodeFile(inputPath)
  local raw, err = ReadFile(inputPath)
  if not raw then
    return nil, err
  end

  local encoded
  encoded, err = ExtractEncodedPayload(raw)
  if not encoded then
    return nil, err
  end

  local payload
  payload, err = DecodePayload(encoded)
  if not payload then
    return nil, err
  end

  return "return " .. Serialize(payload, ""), nil
end

local START_MARKER = "!QuestieTrace:%d+!"
local END_MARKER = "!End:QuestieTrace:%d+!"

--- Decodes a file containing several pasted export strings. A copy that is
--- cut off by the next start marker, or whose payload can't be decoded, is
--- dropped with a warning; identical copies are decoded once.
---@param raw string
---@return string[]|nil exports serialized exports in their original order
---@return string[]|string warnings, or the error when nothing could be decoded
local function DecodeConcatenated(raw)
  local starts = {}
  local searchFrom = 1
  while true do
    local markerStart, markerEnd = raw:find(START_MARKER, searchFrom)
    if (not markerStart) then
      break
    end
    starts[#starts + 1] = { markerStart = markerStart, payloadStart = markerEnd + 1 }
    searchFrom = markerEnd + 1
  end

  local exports, warnings, seenPayloads = {}, {}, {}
  for i, start in ipairs(starts) do
    local nextStart = starts[i + 1] and starts[i + 1].markerStart or math.huge
    local endStart = raw:find(END_MARKER, start.payloadStart)
    if (not endStart) or endStart > nextStart then
      warnings[#warnings + 1] = string.format("export #%d is incomplete and was dropped", i)
    else
      local encoded = raw:sub(start.payloadStart, endStart - 1)
      if (not seenPayloads[encoded]) then
        seenPayloads[encoded] = true
        local payload, err = DecodePayload(encoded)
        if payload then
          exports[#exports + 1] = "return " .. Serialize(payload, "")
        else
          warnings[#warnings + 1] = string.format("export #%d was dropped: %s", i, err)
        end
      end
    end
  end

  if #exports == 0 then
    local reasons = #warnings > 0 and " (" .. table.concat(warnings, "; ") .. ")" or ""
    return nil, "no complete export could be decoded from " .. #starts .. " concatenated export(s)" .. reasons
  end
  return exports, warnings
end

---@param serialized string
---@param outputPath string
---@return boolean ok
---@return string|nil err
local function WriteToFile(serialized, outputPath)
  local file, openErr = io.open(outputPath, "w")
  if not file then
    return false, "cannot open output file '" .. outputPath .. "': " .. tostring(openErr)
  end

  local writeOk, writeErr = file:write(serialized)
  local closeOk, closeErr = file:close()

  if not writeOk then
    return false, "cannot write output file '" .. outputPath .. "': " .. tostring(writeErr)
  end
  if not closeOk then
    return false, "cannot close output file '" .. outputPath .. "': " .. tostring(closeErr)
  end

  return true, nil
end

---@param basename string
---@return string
local function ReplaceExtensionWithLua(basename)
  return (basename:gsub("%.[^.]*$", "")) .. ".lua"
end

---@param inputPath string
---@param outputPath string|nil
local function RunSingleFile(inputPath, outputPath)
  local serialized, err = DecodeFile(inputPath)
  if not serialized then
    Fail(err)
  end

  if not outputPath then
    print(serialized)
    return
  end

  if PathMode(outputPath) == "directory" then
    local basename = inputPath:match("([^/]+)$") or inputPath
    outputPath = outputPath .. "/" .. ReplaceExtensionWithLua(basename)
  end

  local ok, writeErr = WriteToFile(serialized, outputPath)
  if not ok then
    Fail(writeErr)
  end
end

---@param dirPath string
---@return string[]
local function ListTxtFiles(dirPath)
  local files = {}
  for name in lfs.dir(dirPath) do
    if name ~= "." and name ~= ".." and HasTxtExtension(name) then
      local fullPath = dirPath .. "/" .. name
      if PathMode(fullPath) == "file" then
        files[#files + 1] = name
      end
    end
  end
  table.sort(files)
  return files
end

---@param inputDir string
---@param outputDir string
local function RunBatch(inputDir, outputDir)
  EnsureDirectoryExists(inputDir)
  EnsureDirectoryExists(outputDir)

  local files = ListTxtFiles(inputDir)
  if #files == 0 then
    print("no .txt files found in '" .. inputDir .. "'")
    return
  end

  local total = #files
  local succeeded, failed, skippedExisting, recovered = 0, 0, 0, 0
  local usedOutputPaths = {}
  local failures = {}
  local warnings = {}
  for i, name in ipairs(files) do
    local outputName = ReplaceExtensionWithLua(name)
    local outputPath = outputDir .. "/" .. outputName

    if PathMode(outputPath) == "file" then
      skippedExisting = skippedExisting + 1
    else
      local inputPath = inputDir .. "/" .. name
      io.stderr:write("decoding " .. name .. "\n")
      local serialized, err = DecodeFile(inputPath)
      if not serialized then
        -- Check for concatenated export strings (multiple exports in one file)
        local raw, _ = ReadFile(inputPath)
        local isConcat = false
        if raw then
          local startCount = 0
          for _ in raw:gmatch("!QuestieTrace:%d+!") do startCount = startCount + 1 end
          if startCount > 1 then isConcat = true end
        end
        if isConcat then
          local exports, concatResult = DecodeConcatenated(raw)
          local clashingName = usedOutputPaths[outputPath]
          if (not exports) then
            io.stderr:write("skip " .. name .. ": " .. concatResult .. "\n")
            failures[#failures + 1] = name .. ": " .. concatResult
            failed = failed + 1
          elseif clashingName then
            io.stderr:write("skip " .. name .. ": output path '" .. outputPath ..
              "' collides with '" .. clashingName .. "'\n")
            failures[#failures + 1] = name .. ": output path collides with '" .. clashingName .. "'"
            failed = failed + 1
          else
            -- <name>.lua is written last: it marks the file as done for reruns.
            local baseName = outputName:gsub("%.lua$", "")
            local writeErr
            local exportPaths = {}
            for index = 1, #exports do
              local exportPath = index == 1 and outputPath or string.format("%s/%s-%d.lua", outputDir, baseName, index)
              exportPaths[index] = exportPath
              if index > 1 then
                if PathMode(exportPath) then
                  writeErr = "output path '" .. exportPath .. "' already exists"
                elseif usedOutputPaths[exportPath] then
                  writeErr = "output path '" .. exportPath .. "' collides with '" .. usedOutputPaths[exportPath] .. "'"
                end
                if writeErr then
                  break
                end
              end
            end
            local attempted = {}
            if (not writeErr) then
              for index = #exports, 1, -1 do
                attempted[#attempted + 1] = exportPaths[index]
                local ok, exportErr = WriteToFile(exports[index], exportPaths[index])
                if (not ok) then
                  writeErr = exportErr
                  break
                end
              end
            end
            if writeErr then
              for _, path in ipairs(attempted) do
                os.remove(path)
              end
              io.stderr:write("skip " .. name .. ": " .. writeErr .. "\n")
              failures[#failures + 1] = name .. ": " .. writeErr
              failed = failed + 1
            else
              for _, warning in ipairs(concatResult) do
                warnings[#warnings + 1] = name .. ": " .. warning
              end
              for _, path in ipairs(exportPaths) do
                usedOutputPaths[path] = name
              end
              recovered = recovered + 1
            end
          end
        else
          io.stderr:write("skip " .. name .. ": " .. err .. "\n")
          failures[#failures + 1] = name .. ": " .. err
          failed = failed + 1
        end
      else
        local clashingName = usedOutputPaths[outputPath]
        if clashingName then
          io.stderr:write("skip " .. name .. ": output path '" .. outputPath ..
            "' collides with '" .. clashingName .. "'\n")
          failures[#failures + 1] = name .. ": output path collides with '" .. clashingName .. "'"
          failed = failed + 1
        else
          local ok, writeErr = WriteToFile(serialized, outputPath)
          if not ok then
            io.stderr:write("skip " .. name .. ": " .. writeErr .. "\n")
            failures[#failures + 1] = name .. ": " .. writeErr
            failed = failed + 1
          else
            usedOutputPaths[outputPath] = name
            succeeded = succeeded + 1
          end
        end
      end
    end

    if total > 0 and i % 500 == 0 or i == total then
      io.stderr:write(string.format("decode: %d / %d - skipped %d\n", i, total, skippedExisting))
    end
  end

  print(string.format("decoded %d file(s), %d recovered from concatenated exports, %d failed, %d skipped (already in output)",
    succeeded, recovered, failed, skippedExisting))
  if #warnings > 0 then
    print(string.format("%d warning(s):", #warnings))
    for _, warning in ipairs(warnings) do
      print("  " .. warning)
    end
  end
  if #failures > 0 then
    print(string.format("%d file(s) not decoded:", #failures))
    for _, failure in ipairs(failures) do
      print("  " .. failure)
    end
  end
  if failed > 0 then
    os.exit(1)
  end
end

local function Main()
  local inputPath, outputPath = ParseArgs(arg)

  if not inputPath then
    RunBatch(defaultInputDir, defaultOutputDir)
    return
  end

  if PathMode(inputPath) == nil then
    Fail("input path '" .. inputPath .. "' does not exist")
  end

  if not HasTxtExtension(inputPath) then
    Fail("input file '" .. inputPath .. "' must have a .txt extension")
  end

  RunSingleFile(inputPath, outputPath)
end

-- Skip auto-running when loaded by the test suite (which sets _TEST and only
-- wants access to the functions below, not to run the CLI / call os.exit).
-- luacheck: globals _TEST
if not _TEST then
  Main()
  return
end

return {
  ExtractEncodedPayload = ExtractEncodedPayload,
  ReplaceExtensionWithLua = ReplaceExtensionWithLua,
  Serialize = Serialize,
  DecodePayload = DecodePayload,
  DecodeFile = DecodeFile,
  DecodeConcatenated = DecodeConcatenated,
  WriteToFile = WriteToFile,
  PathMode = PathMode,
  HasTxtExtension = HasTxtExtension,
}
