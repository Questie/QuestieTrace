import { afterEach, describe, expect, it } from "vitest";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import type { ObservedSubmission } from "../observe-submission";
import { ObservedStore, type ReplayedSession } from "./observed-store";
import { observerVersion } from "./observer-version";
import { rcloneSource } from "./source";

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
