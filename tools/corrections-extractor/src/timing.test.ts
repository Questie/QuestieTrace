import { describe, expect, it } from "vitest";
import { formatBytes, formatDuration, timed } from "./timing";

describe("formatDuration", () => {
  it.each([
    [850, "850ms"],
    [12_340, "12.3s"],
    [245_000, "4m 05s"],
    [3_725_000, "1h 02m"],
  ])("should format %d ms as %s", (ms, expected) => {
    expect(formatDuration(ms)).toBe(expected);
  });
});

describe("formatBytes", () => {
  it.each([
    [512, "512 B"],
    [1_500_000, "1.5 MB"],
    [2_340_000_000, "2.34 GB"],
  ])("should format %d bytes as %s", (bytes, expected) => {
    expect(formatBytes(bytes)).toBe(expected);
  });
});

describe("timed", () => {
  it("should log the label with the duration and return the result", async () => {
    const lines: string[] = [];

    const result = await timed((line) => lines.push(line), "Did something", async () => 42);

    expect(result).toBe(42);
    expect(lines).toEqual([expect.stringMatching(/^Did something in \d+ms$/)]);
  });
});
