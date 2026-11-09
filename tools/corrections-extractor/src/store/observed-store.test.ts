import { afterEach, describe, expect, it } from "vitest";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { dirname, join } from "path";
import type { ObservedSubmission } from "../observe-submission";
import { addStagedScalarGroup, addStagedSessions, createAccumulator, observeSession, type StagedSession } from "../extract";
import { addLookupScalarGroup, addStagedLookupSessions, createLookupAccumulator, observeLookupSession } from "../extract/lookups";
import type { Confidence } from "../extract/observation";
import { objectivesRealTraceFixture } from "../extract/observers/quest/__fixtures__/objectives.realTrace.fixture";
import { ObservedStore, type ReplayedSession } from "./observed-store";
import { OBSERVER_SOURCES, observerVersion } from "./observer-version";
import { rcloneSource, sampledSource, samplePercentFromEnv } from "./source";

const tempDirs: string[] = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function tempDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "observed-store-"));
  tempDirs.push(dir);
  return dir;
}

function observed(sessions: ObservedSubmission["sessions"]): ObservedSubmission {
  return { submissionId: "id", exportCount: 1, droppedExports: [], sessions };
}

const npcName = { entityId: 7, value: "Gruk", confidence: "high" as const, provenance: { session: "s", t: 1.5 } };

async function replay(store: ObservedStore): Promise<ReplayedSession[]> {
  const sessions: ReplayedSession[] = [];
  await store.replay((session) => sessions.push(session));
  return sessions;
}

describe("ObservedStore", () => {
  it("should replay stored observations with their session identity", async () => {
    const store = await ObservedStore.open(tempDir(), "v1");
    await store.add([
      {
        key: "2026-10/a.json",
        submission: observed([
          { key: "k1", extract: { sessionKey: "k1", contributorId: "c1", sessionTime: 100, scalars: [["npcName", [npcName]]], observations: [] } },
        ]),
      },
    ]);

    const [session] = await replay(store);
    store.close();

    expect(session.extract).toEqual({ sessionKey: "k1", contributorId: "c1", sessionTime: 100, scalars: [["npcName", [npcName]]], observations: [] });
  });

  it("should replay observations exactly as they were added, including extra fields like spawn tokens", async () => {
    const store = await ObservedStore.open(tempDir(), "v1");
    const spawn = { entityId: 9, value: { x: 1, y: 2 }, confidence: "medium" as const, provenance: { session: "s", t: 3 }, token: "questnpc" };
    const extract = {
      sessionKey: "k1",
      sessionTime: null,
      scalars: [["questName", [npcName]], ["npcName", [npcName, { ...npcName, value: "Grak" }]]] as StagedSession["scalars"],
      observations: [["npcSpawns", [spawn]]] as StagedSession["observations"],
    };
    await store.add([{ key: "2026-10/a.json", submission: observed([{ key: "k1", extract }]) }]);

    const [session] = await replay(store);
    store.close();

    expect(session.extract).toEqual(extract);
  });

  it("should replay a real session's observations unchanged", async () => {
    const extract = { ...observeSession(objectivesRealTraceFixture, { npc: 0, quest: 0, item: 0, object: 0 })!, sessionKey: "k1" };
    const lookup = { ...observeLookupSession(objectivesRealTraceFixture)!, sessionKey: "k1" };
    const nonEmpty = <T,>(fields: [string, T[]][]) => fields.filter(([, list]) => list.length > 0);
    const store = await ObservedStore.open(tempDir(), "v1");
    await store.add([{ key: "2026-10/a.json", submission: observed([{ key: "k1", extract, lookup }]) }]);

    const [session] = await replay(store);
    store.close();

    expect(session.extract).toEqual({ ...extract, scalars: nonEmpty(extract.scalars), observations: nonEmpty(extract.observations) });
    expect(session.lookup).toEqual({ ...lookup, observations: nonEmpty(lookup.observations) });
  });

  it("should summarize scalar and lookup values exactly like folding them one by one", async () => {
    const real = { ...observeSession(objectivesRealTraceFixture, { npc: 0, quest: 0, item: 0, object: 0 })!, sessionKey: "k1" };
    const realLookup = { ...observeLookupSession(objectivesRealTraceFixture)!, sessionKey: "k1" };
    const obs = (entityId: number, value: unknown, confidence: Confidence, t: number) => ({ entityId, value, confidence, provenance: { session: "s", t } });
    const synthetic = (key: string, sessionTime: number | null, names: [number, string, Confidence, number][]) => ({
      key,
      extract: { sessionKey: key, sessionTime, scalars: [["npcName", names.map(([id, name, confidence, t]) => obs(id, name, confidence, t))]], observations: [] },
      lookup: { sessionKey: key, sessionTime, observations: [["npcName", names.map(([id, name, confidence, t]) => obs(id, { locale: t % 2 ? "deDE" : "frFR", value: name }, confidence, t))]] },
    }) as ObservedSubmission["sessions"][number];
    const store = await ObservedStore.open(tempDir(), "v1");
    await store.add([
      { key: "2026-10/b.json", submission: observed([{ key: "k1", extract: real, lookup: realLookup }, synthetic("k2", 5, [[2, "B", "low", 1], [1, "A", "high", 2], [2, "A", "low", 3]])]) },
      { key: "2026-09/a.json", submission: observed([synthetic("k3", null, [[1, "A", "high", 1], [2, "B", "low", 4], [1, "A", "high", 6]]), synthetic("k2", 9, [[1, "old", "high", 1]])]) },
    ]);

    const folded = createAccumulator();
    const foldedLookups = createLookupAccumulator();
    await store.replay((session) => {
      if (session.extract) addStagedSessions(folded, [session.extract]);
      if (session.lookup) addStagedLookupSessions(foldedLookups, [session.lookup]);
    });
    const summarized = createAccumulator();
    const summarizedLookups = createLookupAccumulator();
    await store.scalarGroups(({ lookup, field, locale, entityId, group }) => {
      if (lookup) addLookupScalarGroup(summarizedLookups, field, locale!, entityId, group);
      else addStagedScalarGroup(summarized, field, entityId, group);
    });
    store.close();

    // Maps as nested entry lists, so their order is compared too; lastSessionKey only matters while folding.
    const plain = (value: unknown): unknown =>
      value instanceof Map ? [...value].map(([key, entry]) => [key, plain(entry)]) : value && typeof value === "object" && "lastSessionKey" in value ? { ...value, lastSessionKey: undefined } : value;
    expect(plain(new Map(Object.entries(summarized.scalars)))).toEqual(plain(new Map(Object.entries(folded.scalars))));
    expect(plain(new Map(Object.entries(summarizedLookups.summaries)))).toEqual(plain(new Map(Object.entries(foldedLookups.summaries))));
  });

  it("should replay every copy of sessions without a key", async () => {
    const store = await ObservedStore.open(tempDir(), "v1");
    const keyless = { key: undefined, extract: { sessionTime: null, scalars: [], observations: [] } };
    await store.add([{ key: "2026-09/a.json", submission: observed([keyless]) }]);
    await store.add([{ key: "2026-10/b.json", submission: observed([keyless, keyless]) }]);

    const sessions = await replay(store);
    store.close();

    expect(sessions.map((session) => session.submissionKey)).toEqual(["2026-10/b.json", "2026-10/b.json", "2026-09/a.json"]);
  });

  it("should name the submissions holding unobserved copies of a removed submission's sessions", async () => {
    const store = await ObservedStore.open(tempDir(), "v1");
    const used = { key: "k1", extract: { sessionTime: null, scalars: [], observations: [] } };
    await store.add([
      { key: "2026-10/new.json", submission: observed([used]) },
      { key: "2026-09/copy.json", submission: observed([{ key: "k1", duplicate: true }]) },
      { key: "2026-09/other.json", submission: observed([{ key: "k2", duplicate: true }]) },
    ]);

    const holding = await store.holdingCopiesOf(["2026-10/new.json"]);
    store.close();

    expect(holding).toEqual(["2026-09/copy.json"]);
  });

  it("should replay in fold order and keep the newest copy when replaying a slice at a time", async () => {
    const store = await ObservedStore.open(tempDir(), "v1");
    const session = (key: string, name: string) => ({ key, extract: { sessionKey: key, sessionTime: null, scalars: [["npcName", [{ ...npcName, value: name }]]], observations: [] } }) as ObservedSubmission["sessions"][number];
    await store.add([{ key: "2026-09/a.json", submission: observed([session("k1", "old"), session("k2", "a")]) }]);
    await store.add([
      { key: "2026-10/c.json", submission: observed([session("k1", "new")]) },
      { key: "2026-10/b.json", submission: observed([session("k3", "b")]) },
    ]);

    const sessions: ReplayedSession[] = [];
    const progress: number[] = [];
    await store.replay((replayed) => sessions.push(replayed), { onProgress: (done) => progress.push(done), slice: 1 });
    store.close();

    expect(sessions.map((replayed) => replayed.extract?.scalars[0][1][0].value)).toEqual(["new", "b", "a"]);
    expect(progress).toEqual([1, 2, 3]);
  });

  it("should replay only the newest copy of a session", async () => {
    const store = await ObservedStore.open(tempDir(), "v1");
    const copy = (name: string) => observed([{ key: "k1", extract: { sessionKey: "k1", sessionTime: null, scalars: [["npcName", [{ ...npcName, value: name }]]], observations: [] } }]);
    await store.add([{ key: "2026-09/a.json", submission: copy("old") }]);
    await store.add([{ key: "2026-10/b.json", submission: copy("new") }]);

    const sessions: ReplayedSession[] = [];
    const { duplicateCopies } = await store.replay((session) => sessions.push(session));
    store.close();

    expect(sessions.map((session) => session.extract?.scalars[0][1][0].value)).toEqual(["new"]);
    expect(duplicateCopies).toBe(1);
  });

  it("should drop the stored observations when the observer version changes", async () => {
    const dir = tempDir();
    const store = await ObservedStore.open(dir, "v1");
    await store.add([{ key: "2026-10/a.json", submission: observed([{ key: "k1" }]) }]);
    store.close();

    const reopened = await ObservedStore.open(dir, "v2");
    const sessions = await replay(reopened);
    reopened.close();

    expect(reopened.keys().size).toBe(0);
    expect(sessions).toEqual([]);
    expect(readdirSync(join(dir, "batches"))).toEqual([]);
  });
});

describe("observerVersion", () => {
  it("should hash every source the observe path imports", () => {
    const srcDir = join(__dirname, "..");
    const covered = (file: string) => OBSERVER_SOURCES.some((source) => file === source || file.startsWith(`${source}/`));
    const seen = new Set<string>();
    const stack = ["observe-submission.ts", "observe-worker.ts"];
    while (stack.length > 0) {
      const file = stack.pop()!;
      if (seen.has(file)) continue;
      seen.add(file);
      for (const [, specifier] of readFileSync(join(srcDir, file), "utf8").matchAll(/from\s+["'](\.[^"']+)["']/g)) {
        const base = join(dirname(file), specifier);
        const resolved = [`${base}.ts`, join(base, "index.ts")].find((candidate) => existsSync(join(srcDir, candidate)));
        if (resolved) stack.push(resolved);
      }
    }

    expect([...seen].filter((file) => !covered(file))).toEqual([]);
  });

  it("should change when an observer source changes, but not for tests", () => {
    const root = tempDir();
    mkdirSync(join(root, "observers"));
    writeFileSync(join(root, "observers", "name.ts"), "a");
    const before = observerVersion(root, ["observers"]);

    writeFileSync(join(root, "observers", "name.test.ts"), "test");
    expect(observerVersion(root, ["observers"])).toBe(before);

    writeFileSync(join(root, "observers", "name.ts"), "b");
    expect(observerVersion(root, ["observers"])).not.toBe(before);
  });
});

describe("sampledSource", () => {
  const keys = Array.from({ length: 1000 }, (_, i) => `2026-10/${i}.json`);
  const source = { list: async () => keys, fetch: async () => "", release: () => undefined };

  it("should keep about the given percentage of submissions, the same ones every time", async () => {
    const sample = await sampledSource(source, 10).list();

    expect(sample.length).toBeGreaterThan(70);
    expect(sample.length).toBeLessThan(130);
    expect(await sampledSource(source, 10).list()).toEqual(sample);
  });

  it("should only add submissions when the percentage rises", async () => {
    const small = await sampledSource(source, 5).list();
    const large = new Set(await sampledSource(source, 20).list());

    expect(small.every((key) => large.has(key))).toBe(true);
  });
});

describe("samplePercentFromEnv", () => {
  it("should read EXTRACT_SAMPLE as a percentage", () => {
    expect(samplePercentFromEnv({})).toBeUndefined();
    expect(samplePercentFromEnv({ EXTRACT_SAMPLE: "5" })).toBe(5);
    expect(samplePercentFromEnv({ EXTRACT_SAMPLE: "0.5" })).toBe(0.5);
  });

  it("should reject values that are not a percentage above 0 and at most 100", () => {
    for (const value of ["0", "101", "-1", "abc", "5%"]) {
      expect(() => samplePercentFromEnv({ EXTRACT_SAMPLE: value })).toThrow(/EXTRACT_SAMPLE/);
    }
  });
});

describe("rcloneSource", () => {
  /** A stand-in for rclone that serves `remoteDir` for lsf and copy. */
  function fakeRclone(remoteDir: string): string {
    const script = join(tempDir(), "rclone");
    writeFileSync(
      script,
      `#!/bin/sh
case "$1" in
  lsf) cd "${remoteDir}/submissions" && find . -type f | sed 's|^./||' ;;
  copy) while read -r key; do mkdir -p "$(dirname "$9/$key")"; cp "${remoteDir}/submissions/$key" "$9/$key"; done < "$3" ;;
esac
`,
    );
    chmodSync(script, 0o755);
    return script;
  }

  it("should list the remote's submissions and fetch them into a directory that release removes", async () => {
    const remoteDir = tempDir();
    mkdirSync(join(remoteDir, "submissions", "2026-10"), { recursive: true });
    writeFileSync(join(remoteDir, "submissions", "2026-10", "a.json"), "{}");
    const source = rcloneSource("remote:trace-data", fakeRclone(remoteDir));

    expect(await source.list()).toEqual(["2026-10/a.json"]);
    const dir = await source.fetch(["2026-10/a.json"]);
    expect(existsSync(join(dir, "2026-10", "a.json"))).toBe(true);
    source.release(dir);
    expect(existsSync(dir)).toBe(false);
  });
});
