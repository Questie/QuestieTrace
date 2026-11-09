import { describe, expect, it } from "vitest";
import { apiSample, makeSession } from "../test/fixtures";
import { sessionClient } from "./observe";

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
