# QuestieTrace

[![Discord](https://img.shields.io/badge/discord-Questie-738bd7)](https://discord.gg/s33MAYKeZd)

QuestieTrace records a WoW Classic play session to disk — every tracked game event, plus what the WoW API returned at each moment — so the session can be replayed and inspected later. Built for Questie development and debugging.

## Install

**Requirements:** WoW Classic (all flavors: Classic Era, TBC, Wrath, Cata, MoP). Not compatible with Retail.

Copy or symlink this repo into your Classic client's AddOns folder, named exactly `QuestieTrace`:

```
World of Warcraft/<client folder>/Interface/AddOns/QuestieTrace/
```

The folder name must match the `.toc` file name.

## Privacy

QuestieTrace filters out player names, realm names, and player GUIDs before they are recorded or exported. See [`AGENTS.md`](./AGENTS.md) for the complete privacy policy.

## Tools

- **[Trace Analyzer](./tools/trace-analyzer/README.md)** — Local web app to inspect recorded sessions. Pick a trace, scrub a timeline, see what any API call returned at that instant.
- **[Decoder](./tools/decoder/README.md)** — CLI tool to decode export strings back into Lua tables without the analyzer.

## Sharing a Trace

Export your session with `/qlt export` to get a shareable string, then submit it to [questie.dev/trace](https://questie.dev/trace).

## Slash Commands

| Command               | Description                           |
|-----------------------|---------------------------------------|
| `/qlt` or `/qlt help` | Print the command list                |
| `/qlt status`         | Show capture status                   |
| `/qlt tracking`       | Toggle auto-start on login            |
| `/qlt consent`        | Open the consent pop-up               |
| `/qlt debug`          | Toggle debug prints                   |
| `/qlt export [all]`   | Open the export window                |
| `/qlt dumpmap`        | Refresh the static map hierarchy dump |

## Contributing

Architecture and data format are documented in [`specs/README.md`](./specs/README.md). Code style and conventions are in [`AGENTS.md`](./AGENTS.md).

Before opening a PR, run:

```sh
lua Tests/run.lua
busted -p ".test.lua" .
luacheck -q .
```
