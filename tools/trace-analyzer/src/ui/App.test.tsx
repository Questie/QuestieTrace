import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "./App";

const FILES = [
  { name: "2026-11-07_120000Z_a.json", sessionCount: null, error: null },
  { name: "2026-11-08_090000Z_b.json", sessionCount: null, error: null },
];

/** Answers /api/files at once and holds every other request until `release` is called. */
function stubFetch() {
  const pending: (() => void)[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string) => {
      if (url === "/api/files") return Promise.resolve({ json: () => Promise.resolve(FILES) } as Response);
      const body = url.endsWith("/sessions") ? [{ index: 0, name: "s", duration: 10, startedAt: 0, eventCount: 0, functionCount: 0 }] : { error: "not in this test" };
      return new Promise<Response>((resolve) => pending.push(() => resolve({ json: () => Promise.resolve(body) } as Response)));
    }),
  );
  return { release: () => pending.splice(0).forEach((resolve) => resolve()) };
}

describe("App", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("should not open a submission before one is picked", async () => {
    stubFetch();
    render(<App />);

    await screen.findByRole("combobox");

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("should show that a picked submission is downloading, then that its session is loading", async () => {
    const { release } = stubFetch();
    render(<App />);
    fireEvent.focus(await screen.findByRole("combobox"));

    fireEvent.mouseDown(screen.getByText("2026-11-08_090000Z_b.json"));

    expect(await screen.findByRole("status")).toHaveTextContent("Downloading submission...");
    release();
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Loading session..."));
  });
});
