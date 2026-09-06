/// <reference types="node" />

import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const assets = resolve("assets/icons");
const spritePath = resolve("public/sprite.svg");
const icons = readdirSync(assets).filter((file) => file.endsWith(".svg"));

describe("generated icon sprite", () => {
  it.each(icons)("preserves the SVG styling of %s", (file) => {
    const parser = new DOMParser();
    const source = parser.parseFromString(
      readFileSync(join(assets, file), "utf8"),
      "image/svg+xml",
    ).documentElement;
    const sprite = parser.parseFromString(readFileSync(spritePath, "utf8"), "image/svg+xml");
    const symbol = sprite.getElementById(file.slice(0, -4));

    expect(sprite.querySelector("parsererror")).toBeNull();
    expect(symbol).not.toBeNull();
    for (const attribute of [
      "viewBox",
      "fill",
      "stroke",
      "stroke-width",
      "stroke-linecap",
      "stroke-linejoin",
    ]) {
      const value = source.getAttribute(attribute);
      if (value !== null) expect(symbol?.getAttribute(attribute)).toBe(value);
    }
    expect(symbol?.children.length).toBe(source.children.length);
  });
});
