import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterEach, describe, expect, it } from "vitest";
import type { Observation } from "./observation";
import { ObservationStore } from "./observation-store";

const tempDirs: string[] = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function diskStore(): ObservationStore {
  const dir = mkdtempSync(join(tmpdir(), "observation-store-"));
  tempDirs.push(dir);
  return new ObservationStore(join(dir, "store"));
}

const obs = (entityId: number, value: number): Observation<number> => ({ entityId, value, confidence: "high", provenance: { session: "s", t: value } });

describe("ObservationStore", () => {
  it.each([["in memory", () => new ObservationStore()], ["on disk", diskStore]])(
    "should group observations by entity in the order they were added, field after field (%s)",
    (_, create) => {
      const store = create();
      store.add("spawns", [obs(1, 1), obs(65, 2), obs(1, 3)]);
      store.add("extra", [obs(1, 4)]);

      const groups = new Map([...store.groups<number>(["spawns", "extra"])].flatMap((byEntity) => [...byEntity]));

      expect(groups.get(1)?.map((o) => o.value)).toEqual([1, 3, 4]);
      expect(groups.get(65)?.map((o) => o.value)).toEqual([2]);
    },
  );

  it("should stream turnInPoi in the order it was added", () => {
    const store = diskStore();
    store.add("turnInPoi", [obs(70, 1), obs(3, 2), obs(70, 3)]);

    expect([...store.stream<number>("turnInPoi")].flat().map((o) => o.value)).toEqual([1, 2, 3]);
  });

  it("should forget a cleared field, both buffered and written", () => {
    const store = diskStore();
    store.add("extra", [obs(1, 1)]);
    store.all("extra");
    store.add("extra", [obs(2, 2)]);

    store.clear("extra");
    store.add("extra", [obs(3, 3)]);

    expect(store.all<number>("extra").map((o) => o.value)).toEqual([3]);
  });

  it("should remove its files when disposed", () => {
    const store = diskStore();
    store.add("spawns", [obs(1, 1)]);
    store.dispose();

    expect(store.all("spawns")).toEqual([]);
  });
});
