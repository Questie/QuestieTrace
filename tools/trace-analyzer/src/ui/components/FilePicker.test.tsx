import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { FilePicker } from "./FilePicker";

const FILES = ["2026-10-01_000000Z_a.json", "2026-11-07_120000Z_b.json", "2026-11-08_090000Z_c.json"];

function open(files = FILES, onSelect = vi.fn()) {
  render(<FilePicker files={files} value={null} onSelect={onSelect} />);
  const input = screen.getByRole("combobox");
  fireEvent.focus(input);
  return { input, onSelect };
}

describe("FilePicker", () => {
  it("should list files newest first when opened", () => {
    open();

    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual([...FILES].reverse());
  });

  it("should only list files matching every typed term", () => {
    const { input } = open();

    fireEvent.change(input, { target: { value: "2026-11 b" } });

    expect(screen.getAllByRole("option").map((option) => option.textContent)).toEqual(["2026-11-07_120000Z_b.json"]);
  });

  it("should list at most 100 matches and say how many more there are", () => {
    const many = Array.from({ length: 250 }, (_, i) => `2026-11-01_${String(i).padStart(6, "0")}Z_x.json`);
    open(many);

    expect(screen.getAllByRole("option")).toHaveLength(100);
    expect(screen.getByText(/150 more/)).toBeInTheDocument();
  });

  it("should select a clicked file", () => {
    const { onSelect } = open();

    fireEvent.mouseDown(screen.getByText("2026-10-01_000000Z_a.json"));

    expect(onSelect).toHaveBeenCalledWith("2026-10-01_000000Z_a.json");
    expect(screen.queryByRole("option")).toBeNull();
  });

  it("should select the highlighted file with the arrow keys and Enter", () => {
    const { input, onSelect } = open();

    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(onSelect).toHaveBeenCalledWith("2026-11-07_120000Z_b.json");
  });

  it("should close without selecting on Escape", () => {
    const { input, onSelect } = open();

    fireEvent.keyDown(input, { key: "Escape" });

    expect(screen.queryByRole("option")).toBeNull();
    expect(onSelect).not.toHaveBeenCalled();
  });
});
