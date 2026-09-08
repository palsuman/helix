import { render, screen } from "@testing-library/react";
import { useEffect, useRef, useState } from "react";
import { describe, expect, it } from "vitest";
import { configureAxe } from "vitest-axe";
import {
  assertEscapeRestoresFocus,
  assertFocusTrap,
  assertTabOrder,
  activateWithKeyboard,
  expectContrast,
  pressKey,
} from "./accessibility";

const runAxe = configureAxe({ rules: { "color-contrast": { enabled: false } } });

function AccessibleDialog() {
  const [open, setOpen] = useState(false);
  const [activated, setActivated] = useState(0);
  const [alignment, setAlignment] = useState<"left" | "right">("left");
  const triggerRef = useRef<HTMLButtonElement>(null);
  const firstRef = useRef<HTMLButtonElement>(null);
  const lastRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) firstRef.current?.focus();
  }, [open]);

  return (
    <main>
      <button ref={triggerRef} onClick={() => setOpen(true)}>
        Open settings
      </button>
      <button onClick={() => setActivated((value) => value + 1)}>Toolbar action {activated}</button>
      <output aria-label="Activation count">{activated}</output>
      <div
        aria-label="Alignment"
        onKeyDown={(event) => {
          if (event.key === "ArrowRight") setAlignment("right");
          if (event.key === "ArrowLeft") setAlignment("left");
        }}
        role="group"
        tabIndex={0}
      >
        <button aria-pressed={alignment === "left"}>Left</button>
        <button aria-pressed={alignment === "right"}>Right</button>
      </div>
      {open ? (
        <section
          aria-labelledby="dialog-title"
          aria-modal="true"
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setOpen(false);
              requestAnimationFrame(() => triggerRef.current?.focus());
            }
            if (event.key !== "Tab") return;
            if (event.shiftKey && document.activeElement === firstRef.current) {
              event.preventDefault();
              lastRef.current?.focus();
            } else if (!event.shiftKey && document.activeElement === lastRef.current) {
              event.preventDefault();
              firstRef.current?.focus();
            }
          }}
          role="dialog"
        >
          <h1 id="dialog-title">Settings</h1>
          <button ref={firstRef} onClick={() => setActivated((value) => value + 1)}>
            Save settings
          </button>
          <button ref={lastRef} onClick={() => setOpen(false)}>
            Close settings
          </button>
        </section>
      ) : null}
    </main>
  );
}

describe("accessibility harness", () => {
  it("reports axe violations with their rule ids", async () => {
    const { container } = render(
      <button data-testid="unlabeled-icon">
        <svg aria-hidden="true" />
      </button>,
    );
    const results = await runAxe(container);
    expect(results.violations.map((violation) => violation.id)).toContain("button-name");
  });

  it("passes axe, keyboard, focus, and contrast checks for an accessible dialog", async () => {
    const { container } = render(<AccessibleDialog />);
    expect((await runAxe(container)).violations).toEqual([]);

    const trigger = screen.getByRole("button", { name: "Open settings" });
    activateWithKeyboard(trigger);
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    const dialog = screen.getByRole("dialog");
    const first = screen.getByRole("button", { name: "Save settings" });
    const last = screen.getByRole("button", { name: "Close settings" });
    assertTabOrder(dialog, [first, last]);
    assertFocusTrap(dialog, first, last);
    activateWithKeyboard(first);
    expect(screen.getByRole("status", { name: "Activation count" })).toHaveTextContent("1");

    const alignment = screen.getByRole("group", { name: "Alignment" });
    alignment.focus();
    pressKey(alignment, "ArrowRight");
    expect(screen.getByRole("button", { name: "Right" })).toHaveAttribute("aria-pressed", "true");

    await assertEscapeRestoresFocus(dialog, trigger);
    expectContrast("#202124", "#ffffff");
  });
});