# QuestieTrace

QuestieTrace records a World of Warcraft **Classic** play session to disk — every tracked game event, plus what the WoW API returned at each moment — so the session can be replayed and inspected later without a running game client.

It's built for Questie development: capture a real character doing real quests, then step through exactly what the game told the addon at any point in time.

| Part | What it is | Who needs it |
|---|---|---|
| **The addon** | A WoW addon that records sessions to disk | Everyone |
| **The analyzer** | A local web app that reads those recordings | Anyone inspecting a trace |

---

## Part 1 — The addon

Records the events the game fires and the return values of the WoW API functions Questie depends on — quest log, position, reputation, spellbook, loot, gossip dialogs and more. Values are only stored when they change, and everything is timestamped relative to the start of the session.

**Requirements:** WoW Classic Era. Nothing else.

**Install:** copy or symlink this repo into your AddOns folder:

```
World of Warcraft/_classic_era_/Interface/AddOns/QuestieTrace/
```

The folder must be named exactly `QuestieTrace`, or the game won't find `QuestieTrace-Classic.toc`.

Classic Era is the only supported client. There are no Retail, TBC, Wrath or Cata builds.

**You don't need to do anything to start recording.** The addon begins capturing when you log in and saves automatically when you log out.

---

## Part 2 — The analyzer

A local React app. Pick a session, scrub a timeline, and see what any recorded API call returned at that instant. Three views: **streams** (API return values over time), **events** (the raw event log), and **position** (where the character was on the map).

**Requirements:**

- Node.js 20 or newer
- npm
- No game client and no WoW install needed — it reads trace files, nothing else

**Run it:**

```sh
cd tools/trace-analyzer
npm ci
npm run dev
```

A browser tab opens automatically.

> **Use `npm run dev`, not `npm run build`.** Trace loading runs inside the Vite dev server. A production build has no way to read trace files and will show you nothing.

Want to check your setup before capturing anything? A sample trace ships with the repo — start the analyzer and open `human_rogue_1to5_example.lua`. If that loads, you're ready.

---

## Part 3 — decoder

A small standalone CLI for decoding a QuestieTrace export string back into a plain Lua table, without the analyzer. See [`tools/decoder/README.md`](./tools/decoder/README.md).

---

## Usage guide

### 1. Record

Log in with the addon installed. Recording starts by itself — just play.

To check on it, type `/qlt status` in chat. A small Start/Stop/Save panel is also shown on screen with an "Auto-start on login" checkbox.

### 2. Log out

> **Your session is not on disk until you log out or `/reload`.** WoW only writes addon data at those two moments. Alt-F4 or a crash loses the session. So regular `/reload`s while actively playing is recommended.

### 3. Find your trace file

```
World of Warcraft/_classic_era_/WTF/Account/<ACCOUNT>/<Realm>/<Character>/SavedVariables/QuestieTrace.lua
```

Sessions are saved per character, so make sure you're looking under the character you played.

### 4. Analyze it

Copy that file into the `Traces/` folder in this repo, then start the analyzer. It picks up every `.lua` file in `Traces/` and lists all sessions inside each one.

If you add a file while the analyzer is already running, restart it or visit `/api/reload`.

`Traces/` is gitignored apart from the bundled example, so you can drop captures there freely without dirtying the repo.

### Sending a capture to a developer

Just send the `QuestieTrace.lua` file. Note that it contains your character name, realm, level, everywhere you walked, your quest log and your known spells — nothing private to your account, but worth knowing before you share it.

---

## Slash commands

Both `/qlt` and `/questietrace` work.

| Command | Description |
|---|---|
| `/qlt` or `/qlt help` | Print the command list |
| `/qlt start [name]` | Start a capture, optionally naming it |
| `/qlt stop` | Stop the active capture |
| `/qlt save [name]` | Save the session, stopping it first if needed |
| `/qlt reset` | Discard an unsaved session (only when stopped) |
| `/qlt status` | Print capture status to chat |
| `/qlt auto` | Toggle auto-start on login |
| `/qlt dumpmap` | Dump the map hierarchy (provided by the map dump provider) |

Sessions are named by date and time unless you give one. The oldest are pruned once you pass 20 saved sessions.

---

## Troubleshooting

**The addon isn't in my addon list.** The folder has to be named exactly `QuestieTrace`. Also tick "Load out of date addons" on the character select screen.

**There's no `QuestieTrace.lua` in SavedVariables.** You haven't logged out or `/reload`ed since recording. See step 2.

**The analyzer lists no files.** Your capture needs to be in `Traces/` with a `.lua` extension. Restart the dev server after adding it.

**A file shows a load error.** It must be a real QuestieTrace SavedVariables file — the analyzer looks for the `QuestieTraceCharacter` global and fails if it isn't there. Character-specific saves have it; the account-wide `QuestieTrace.lua` settings file does not.

---

## Contributing

Internals, data format and tracker architecture are documented in [`specs/README.md`](./specs/README.md). Build and style conventions are in [`AGENTS.md`](./AGENTS.md).

Before opening a PR:

```sh
lua5.1 Tests/run.lua
luacheck -q -- Trackers globals.lua QuestieTrace.lua QuestieTrace_UI.lua
```
