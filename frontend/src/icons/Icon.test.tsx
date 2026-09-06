import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { Icon } from "./Icon";
import type { IconId } from "../generated/icons.gen";

// Mock the generated icons module
vi.mock("../generated/icons.gen", () => ({
  KNOWN_ICON_IDS: [
    "file",
    "folder",
    "folder-open",
    "add",
    "terminal",
    "split-horizontal",
    "split-vertical",
    "placeholder",
  ],
  ICON_SIZE_PX: { sm: 12, md: 16, lg: 20 },
  RTL_MIRRORED_ICON_IDS: ["split-horizontal"],
  resolveIconId: (id: string) => {
    const known = [
      "file",
      "folder",
      "folder-open",
      "add",
      "terminal",
      "split-horizontal",
      "split-vertical",
      "placeholder",
    ];
    return known.includes(id) ? id : "placeholder";
  },
}));

describe("Icon component", () => {
  it("renders an SVG use element referencing the sprite", () => {
    render(<Icon id="file" />);
    const svg = screen.getByTestId("icon");
    expect(svg).toBeInTheDocument();
    const use = svg.querySelector("use");
    expect(use).toHaveAttribute("href", "/sprite.svg#file");
  });

  it("applies size classes correctly", () => {
    const { rerender } = render(<Icon id="file" size="sm" />);
    expect(screen.getByTestId("icon")).toHaveClass("icon-sm");

    rerender(<Icon id="file" size="md" />);
    expect(screen.getByTestId("icon")).toHaveClass("icon-md");

    rerender(<Icon id="file" size="lg" />);
    expect(screen.getByTestId("icon")).toHaveClass("icon-lg");
  });

  it("adds spin class when spin prop is true", () => {
    render(<Icon id="file" spin />);
    expect(screen.getByTestId("icon")).toHaveClass("icon-spin");
  });

  it("adds custom className", () => {
    render(<Icon id="file" className="custom-class" />);
    expect(screen.getByTestId("icon")).toHaveClass("custom-class");
  });

  it("automatically marks directional icons for RTL mirroring", () => {
    render(<Icon id="split-horizontal" />);
    expect(screen.getByTestId("icon")).toHaveClass("icon-rtl-flip");
  });

  it("sets aria-label and role=img when label is provided", () => {
    render(<Icon id="file" label="Open file" />);
    const svg = screen.getByRole("img", { name: "Open file" });
    expect(svg).toHaveAttribute("aria-label", "Open file");
    expect(svg).toHaveAttribute("role", "img");
    expect(svg.querySelector("title")).toHaveTextContent("Open file");
  });

  it("sets aria-hidden when no label is provided", () => {
    render(<Icon id="file" />);
    const svg = screen.getByTestId("icon");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg).not.toHaveAttribute("aria-label");
  });

  it("renders placeholder for unknown icon IDs", () => {
    render(<Icon id={"unknown-icon" as IconId} />);
    const svg = screen.getByTestId("icon");
    expect(svg).toBeInTheDocument();
    const use = svg.querySelector("use");
    expect(use).toHaveAttribute("href", "/sprite.svg#placeholder");
  });
});
