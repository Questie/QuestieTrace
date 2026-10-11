import { afterEach, describe, expect, it, vi } from "vitest";
import { resolve } from "path";

async function loadPaths() {
  vi.resetModules();
  return (await import("./paths")).paths;
}

describe("paths.outputDir", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("publishes inside a scratch RELATIONS_DIR, never into the real output/", async () => {
    vi.stubEnv("RELATIONS_DIR", "/scratch/relations");
    vi.stubEnv("RELATIONS_OUTPUT_DIR", "");

    expect((await loadPaths()).outputDir).toBe(resolve("/scratch/relations/output"));
  });

  it("prefers RELATIONS_OUTPUT_DIR when set", async () => {
    vi.stubEnv("RELATIONS_DIR", "/scratch/relations");
    vi.stubEnv("RELATIONS_OUTPUT_DIR", "/scratch/deliverables");

    expect((await loadPaths()).outputDir).toBe(resolve("/scratch/deliverables"));
  });
});
