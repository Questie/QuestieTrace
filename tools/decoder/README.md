# decoder

Decodes a QuestieTrace export string (as produced by `Core.BuildExportString`, e.g. via the export UI or `/qlt` export flow) back into a plain Lua table you can read or diff.

It reverses the encoding pipeline in `Modules/Export/Encoding.lua`:

```
EncodeForPrint -> Deflate decompress -> CBOR deserialize
```

## Requirements

- Lua 5.1 (`lua5.1`)
- LuaFileSystem (`lfs`) — same dependency the test suite already uses

## Usage

### Batch mode (default)

Drop one or more export `.txt` files into `input/`, then run the script with no arguments:

```sh
lua5.1 tools/decoder/decoder.lua
```

Every `.txt` file in `input/` is decoded and written to `output/` as `<basename>.lua`. Both directories live next to the script and are created automatically if they don't exist yet — nothing to set up beforehand. Files without a `.txt` extension are ignored.

Files that fail to decode (empty, corrupt, not a QuestieTrace export, ...) are reported to stderr and skipped; the rest of the batch still runs. A summary line (`decoded N file(s), M failed`) is printed at the end, and the process exits with code `1` if anything failed.

### Single file mode

Pass a `.txt` file path to decode just that one file, printed to stdout:

```sh
lua5.1 tools/decoder/decoder.lua path/to/export.txt
```

Files without a `.txt` extension are rejected with an error.

Add `-o <path>` to write the result to a file (or a directory, in which case `<basename>.lua` is used) instead of printing it:

```sh
lua5.1 tools/decoder/decoder.lua path/to/export.txt -o decoded.lua
```

### Help

```sh
lua5.1 tools/decoder/decoder.lua --help
```

## Directories

- `input/` — put export `.txt` files here for batch mode. Gitignored (except for a `.gitkeep` placeholder).
- `output/` — decoded `.lua` files land here. Gitignored (except for a `.gitkeep` placeholder).
- `dependencies/` — vendored decode libraries (`LibDeflate`, `BlizzardCBOR`) this script depends on. Not used anywhere else in the repo.

## Importing from trace-data

`run.sh` automates the full pipeline for turning submissions from the
`trace-data` repo into ready-to-use trace files:

```sh
tools/decoder/run.sh [trace-data-dir]
```

This runs, in order:

1. `extract.lua` — recursively scans `trace-data/submissions/**/*.json`
   (or `[trace-data-dir]` if given) for each submission's `export_string`
   field and writes it to `input/<id>.txt`.
2. `decoder.lua` — batch-decodes everything in `input/` into `output/`, as
   described above.
3. Moves every decoded `output/*.lua` file into `../../Traces/`, rewriting
   its `return { ... }` header to `QuestieTraceCharacter = { ... }` so it
   matches the SavedVariables format `tools/trace-analyzer` expects.
   Existing files with the same name are overwritten.

