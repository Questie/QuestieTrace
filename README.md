# QuestieTrace

QuestieTrace records a WoW Classic play session to disk — every tracked game event, plus what the WoW API returned at each moment — so the session can be replayed and inspected later without a running game client.

It's built for Questie development: capture a real character doing real quests, then step through exactly what the game told the addon at any point in time.

| Part | What it is | Who needs it |
|---|---|---|
| **The addon** | A WoW addon that records sessions to disk | Everyone |
| **The analyzer** | A local web app that reads those recordings | Anyone inspecting a trace |

---

## Part 1 — The addon

Records the events the game fires and the return values of the WoW API functions Questie depends on — quest log, position, reputation, spellbook, loot, gossip dialogs and more. Values are only stored when they change, and everything is timestamped relative to the start of the session.

**Requirements:** WoW Classic. Every Classic flavor is supported — Classic Era (including Anniversary/Fresh realms), TBC, Wrath, Cata and MoP Classic. Retail is not supported.

**Install:** copy or symlink this repo into your Classic client's AddOns folder, named exactly `QuestieTrace`:

```
World of Warcraft/<client folder>/Interface/AddOns/QuestieTrace/
```

The folder name matters — the game finds the addon by matching it to `QuestieTrace-<Flavor>.toc`.

**First login:** you'll be asked for consent to collect data locally. Nothing is recorded before you accept, and declining stops the addon from doing anything. You can change your answer anytime with `/qlt consent`. Once consented, recording is automatic — log in and play.

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

Log in, accept the consent prompt on first login, and play. Recording starts and stops by itself — check `/qlt status` any time to see what's happening.

### 2. Log out

> **Your session is not on disk until you log out or `/reload`.** WoW only writes addon data at those two moments. Alt-F4 or a crash loses the session. So regular `/reload`s while actively playing is recommended.

### 3. Find your trace file

```
World of Warcraft/<client folder>/WTF/Account/<ACCOUNT>/<Realm>/<Character>/SavedVariables/QuestieTrace.lua
```

Sessions are saved per character, so make sure you're looking under the character you played.

### 4. Analyze it

Copy that file into the `Traces/` folder in this repo, then start the analyzer. It picks up every `.lua` file in `Traces/` and lists all sessions inside each one.

If you add a file while the analyzer is already running, restart it or visit `/api/reload`.

`Traces/` is gitignored apart from the bundled example, so you can drop captures there freely without dirtying the repo.

### Sending a capture to a developer

Just send the `QuestieTrace.lua` file, or use `/qlt export` in-game to get a shareable string instead. Either way it contains your character name, realm, level, everywhere you walked, your quest log and your known spells — nothing private to your account, but worth knowing before you share it.

---

## Slash commands

Both `/qlt` and `/questietrace` work.

| Command | Description |
|---|---|
| `/qlt` or `/qlt help` | Print the command list |
| `/qlt status` | Print capture status to chat |
| `/qlt tracking` | Toggle auto-start on login |
| `/qlt consent` | Show the data collection consent prompt |
| `/qlt debug` | Toggle debug prints |
| `/qlt export` | Open the export window |
| `/qlt export all` | Reopen the export window, including previously exported sessions (e.g. if a submission failed) |
| `/qlt clear` | Delete sessions you've already shared, freeing up space |
| `/qlt clear all` | Delete every collected session, including ones you haven't shared yet — asks for confirmation first |
| `/qlt dumpmap` | Refresh the static map hierarchy dump |

Sessions are named by date and time. The oldest are pruned once you pass 20 saved sessions. Opening the export window (`/qlt export`) also clears out whatever was shared in the *previous* export, so collected data never grows past one share cycle — use `/qlt clear all` any time you want to purge everything, shared or not.

**`/qlt export all` only recovers one export back.** It exists to resend a batch that failed to copy/paste, but that recovery window closes the moment you run a plain `/qlt export` again — that call sweeps away the batch `export all` would have recovered. If a share fails, use `/qlt export all` before exporting normally again.

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
lua Tests/run.lua
busted -p ".test.lua" .
luacheck -q .
```
