import { afterEach, describe, expect, it, vi } from "vitest";
import { apiSample, makeSession } from "../test/fixtures";
import { createAccumulator, finalize } from "./index";
import { createLookupAccumulator, finalizeLookups } from "./lookups";
import { isForeverSession, OBSERVERS, SCALAR_OBSERVERS, sessionClient } from "./observe";
import { LOOKUP_OBSERVERS } from "./observe-lookups";

/** Records every key read from `target`. */
function recordingKeys<T extends object>(target: T, used: Set<string>): T {
  return new Proxy(target, {
    get(object, key, receiver) {
      used.add(String(key));
      return Reflect.get(object, key, receiver);
    },
  });
}

describe("registered observers", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  // A field counts as merged when finalize reads it. Store reads go through groups, stream or all.
  it("should all be read by finalize", () => {
    const acc = createAccumulator();
    const used = new Set<string>();
    acc.scalars = recordingKeys(acc.scalars, used);
    const store = acc.observations;
    for (const method of ["groups", "stream", "all"] as const) {
      const original = store[method].bind(store) as (fields: string | string[]) => unknown;
      vi.spyOn(store, method).mockImplementation(((fields: string | string[]) => {
        for (const field of [fields].flat()) used.add(field);
        return original(fields);
      }) as never);
    }

    finalize(acc, { sourceFileNames: [] });

    expect([...Object.keys(SCALAR_OBSERVERS), ...Object.keys(OBSERVERS)].filter((key) => !used.has(key))).toEqual([]);
  });

  it("should all be read by finalizeLookups", () => {
    const acc = createLookupAccumulator();
    // A locale per field, so finalizeLookups reaches every field.
    for (const summaries of Object.values(acc.summaries)) summaries.set("deDE", new Map());
    const used = new Set<string>();
    acc.summaries = recordingKeys(acc.summaries, used);

    finalizeLookups(acc);

    expect(Object.keys(LOOKUP_OBSERVERS).filter((key) => !used.has(key))).toEqual([]);
  });
});

describe("isForeverSession", () => {
  it("should reject a Classic session known only from its top-level buildInfo", () => {
    const session = { ...makeSession({}), buildInfo: { "4": 11508, n: 4 } };

    expect(isForeverSession(session)).toBe(false);
  });

  it("should accept a Forever session known only from its top-level buildInfo", () => {
    const session = { ...makeSession({}), buildInfo: { "4": 16001, n: 4 } };

    expect(isForeverSession(session)).toBe(true);
  });

  it("should prefer the GetBuildInfo stream over the top-level buildInfo", () => {
    const session = {
      ...makeSession({ GetBuildInfo: { player: [apiSample(0, 0, 16001)] } }),
      buildInfo: { "4": 11508, n: 4 },
    };

    expect(isForeverSession(session)).toBe(true);
  });

  it("should treat a top-level buildInfo version that is not a number or string as unavailable", () => {
    for (const version of [true, null, [], {}]) {
      const session = { ...makeSession({}), buildInfo: { "4": version, n: 4 } };

      expect(isForeverSession(session)).toBe(true);
    }
  });

  it("should accept a session that has no version information", () => {
    expect(isForeverSession(makeSession({}))).toBe(true);
  });
});

describe("sessionClient", () => {
  it("should read the interface version, build number and locale of a session", () => {
    const session = {
      ...makeSession({ GetBuildInfo: { player: [apiSample(0, 0, 16001)] }, GetLocale: { player: [apiSample(0, 0, "deDE")] } }),
      buildInfo: { "1": "1.60.1", "2": "70862", "3": "Nov  6 2026", "4": 16001, n: 4 },
    };

    expect(sessionClient(session)).toEqual({ interfaceVersion: 16001, build: 70862, locale: "deDE" });
  });

  it("should leave out what an older session did not record", () => {
    expect(sessionClient(makeSession({}))).toEqual({ interfaceVersion: null, build: null, locale: null });
  });
});
