import { fireEvent } from "@testing-library/react";
import { expect } from "vitest";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function focusableElements(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
}

/** Assert the DOM tab order and make the intended order explicit in tests. */
export function assertTabOrder(root: HTMLElement, expected: HTMLElement[]): void {
  expect(focusableElements(root)).toEqual(expected);
}

export function pressKey(target: HTMLElement, key: "ArrowDown" | "ArrowLeft" | "ArrowRight" | "ArrowUp" | "Enter" | "Escape" | "Space" | "Tab", options: KeyboardEventInit = {}): void {
  fireEvent.keyDown(target, { key: key === "Space" ? " " : key, ...options });
}

export function activateWithKeyboard(target: HTMLElement): void {
  pressKey(target, "Enter");
  fireEvent.keyUp(target, { key: "Enter" });
  pressKey(target, "Space");
  fireEvent.keyUp(target, { key: " " });
  // jsdom does not synthesize the native click that a browser emits for a
  // focused button after Enter or Space.
  fireEvent.click(target);
}

/** Exercise both directions of a modal focus trap. */
export function assertFocusTrap(dialog: HTMLElement, first: HTMLElement, last: HTMLElement): void {
  first.focus();
  pressKey(dialog, "Tab", { shiftKey: true });
  expect(document.activeElement).toBe(last);

  last.focus();
  pressKey(dialog, "Tab");
  expect(document.activeElement).toBe(first);
}

export async function assertEscapeRestoresFocus(
  target: HTMLElement,
  trigger: HTMLElement,
): Promise<void> {
  pressKey(target, "Escape");
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  expect(trigger).toHaveFocus();
}

type Rgb = { blue: number; green: number; red: number };

function parseColor(color: string): Rgb {
  const hex = color.trim().replace(/^#/, "");
  if (/^[0-9a-f]{3}$/i.test(hex)) {
    return {
      red: Number.parseInt(`${hex[0]}${hex[0]}`, 16),
      green: Number.parseInt(`${hex[1]}${hex[1]}`, 16),
      blue: Number.parseInt(`${hex[2]}${hex[2]}`, 16),
    };
  }
  if (/^[0-9a-f]{6}$/i.test(hex)) {
    return {
      red: Number.parseInt(hex.slice(0, 2), 16),
      green: Number.parseInt(hex.slice(2, 4), 16),
      blue: Number.parseInt(hex.slice(4, 6), 16),
    };
  }
  const rgb = color.match(/^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/i);
  if (rgb) {
    return { red: Number(rgb[1]), green: Number(rgb[2]), blue: Number(rgb[3]) };
  }
  throw new Error(`Unsupported color format: ${color}`);
}

function luminance(color: Rgb): number {
  const channels = [color.red, color.green, color.blue].map((channel) => channel / 255);
  const linear = channels.map((channel) =>
    channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

export function contrastRatio(foreground: string, background: string): number {
  const foregroundLuminance = luminance(parseColor(foreground));
  const backgroundLuminance = luminance(parseColor(background));
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

export function expectContrast(
  foreground: string,
  background: string,
  minimumRatio = 4.5,
): void {
  expect(contrastRatio(foreground, background)).toBeGreaterThanOrEqual(minimumRatio);
}