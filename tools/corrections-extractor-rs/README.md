# corrections-extractor-rs

A Rust port of [corrections-extractor](../corrections-extractor/README.md): it reads every submission of a local [trace-data](https://github.com/Questie/trace-data) checkout and writes the same Questie correction and translation modules. See [DESIGN.md](DESIGN.md) for how it is built and what has to match the TypeScript tool.

## Requirements

- Rust 1.82 or newer (`cargo`)

## Usage

```sh
cd tools/corrections-extractor-rs
cargo run --release -- --trace-data ../../../trace-data
```

| Option | Default | |
|---|---|---|
| `--trace-data DIR` | `$TRACE_DATA_DIR` | The trace-data checkout to read |
| `--output DIR` | `output`, or `output/sample-<PERCENT>` | Where the modules go |
| `--sample PERCENT` | `$EXTRACT_SAMPLE` | Only this percentage of submissions, the same ones every run (and the same ones as the TypeScript tool's sample) |
| `--threads N` | one per core | Decoding threads |

Every run reads every submission: there is no incremental store. To work from the OVH bucket, sync it to a local directory first.

The output files are those of the TypeScript tool: `forever{Npc,Quest,Item,Object}Traces.lua`, `xpDB-traces.lua`, `lookup{Npcs,Quests,Items,Objects}/<locale>.lua` and `meta.json`, which is written last and marks a complete run.

## Parity with the TypeScript tool

```sh
TRACE_DATA_DIR=../../../trace-data scripts/parity.sh 1
```

runs both tools on the same 1% sample and diffs every output file; only the generation timestamps may differ. Run it after any change to observers, merges or writers.

## Development

```sh
cargo test
cargo clippy --all-targets
```
