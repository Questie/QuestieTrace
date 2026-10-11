import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join, relative } from "path";
import { afterEach, describe, expect, it } from "vitest";
import { TINY_EXPORT } from "./fixtures";
import { listSubmissionFiles, readSubmission } from "./submissions";

const tempDirs: string[] = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

/** A trace-data checkout holding `files` (path under submissions/ -> content). */
function checkout(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "trace-data-"));
  tempDirs.push(root);
  for (const [path, content] of Object.entries(files)) {
    const full = join(root, "submissions", path);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, content);
  }
  return root;
}

describe("listSubmissionFiles", () => {
  it("lists every month's submissions oldest first, with the id from the file name", () => {
    const root = checkout({
      "2026-10/2026-10-01_000000Z_bbb.json": "{}",
      "2026-09/2026-09-30_235959Z_aaa.json": "{}",
      "2026-10/notes.txt": "",
      "README.md": "",
    });

    const files = listSubmissionFiles(root).map((file) => [relative(join(root, "submissions"), file.path), file.id]);

    expect(files).toEqual([
      ["2026-09/2026-09-30_235959Z_aaa.json", "aaa"],
      ["2026-10/2026-10-01_000000Z_bbb.json", "bbb"],
    ]);
  });
});

describe("readSubmission", () => {
  it("returns the pseudonymous ids and the decoded exports, and nothing else from the file", () => {
    const root = checkout({
      "2026-10/2026-10-01_000000Z_file-id.json": JSON.stringify({
        id: "submission-id",
        contributor_id: "contributor-id",
        received_at: "2026-10-01T00:00:00Z",
        nickname: "SomePlayerName",
        export_string: TINY_EXPORT,
      }),
    });

    const submission = readSubmission(listSubmissionFiles(root)[0].path);
    const { traces, ...rest } = submission;

    expect(rest).toEqual({ submissionId: "submission-id", contributorId: "contributor-id", receivedAt: "2026-10-01T00:00:00Z" });
    expect(traces.map((trace) => trace.sessions.length)).toEqual([1]);
    expect(JSON.stringify(submission)).not.toContain("SomePlayerName");
  });

  it("reports a submission it cannot read without quoting it, keeping the file name's id", () => {
    const root = checkout({
      "2026-10/2026-10-01_000000Z_broken.json": '{"nickname": "SomePlayerName", ',
      "2026-10/2026-10-02_000000Z_garbled.json": JSON.stringify({ id: "garbled", export_string: "not an export" }),
    });
    const [broken, garbled] = listSubmissionFiles(root).map((file) => readSubmission(file.path));

    expect(broken).toEqual({ submissionId: "broken", contributorId: "", receivedAt: "", traces: [], error: "unreadable submission JSON" });
    expect(garbled.submissionId).toBe("garbled");
    expect(garbled.traces).toEqual([]);
    expect(garbled.error).toMatch(/markers/);
  });
});
