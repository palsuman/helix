import { describe, expect, it } from "vitest";
import ar from "./locales/ar.json";
import de from "./locales/de.json";
import en from "./locales/en.json";
import es from "./locales/es.json";
import fr from "./locales/fr.json";
import ja from "./locales/ja.json";
import { messages } from "./messages";

describe("localization catalogs", () => {
  it("contains every extracted descriptor exactly once in the English base catalog", () => {
    const descriptorIds = Object.values(messages)
      .map((message) => String(message.id))
      .sort();
    expect(new Set(descriptorIds).size).toBe(descriptorIds.length);
    expect(Object.keys(en).sort()).toEqual(descriptorIds);
    expect(Object.values(en).every((value) => value.trim() !== "")).toBe(true);
  });

  it.each(Object.entries({ ar, de, es, fr, ja }))(
    "%s contains only known, nonblank messages",
    (_locale, catalog) => {
      expect(Object.keys(catalog).every((id) => id in en)).toBe(true);
      expect(Object.values(catalog).every((value) => value.trim() !== "")).toBe(true);
    },
  );
});
