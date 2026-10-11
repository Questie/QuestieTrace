import { tmpdir } from "os";
import { join } from "path";
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    globals: false,
    // The relations pipeline writes its cache and deliverables relative to RELATIONS_DIR
    // (src/relations/core/paths.ts). Pointing it at a temp dir means no test can ever touch the
    // real .relations/ cache or the published output/ files.
    env: { RELATIONS_DIR: join(tmpdir(), "corrections-extractor-vitest") },
  },
});
