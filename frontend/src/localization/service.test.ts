import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { LocalizationService, localeDirection, negotiateLocale, PSEUDO_LOCALE } from "./service";

describe("LocalizationService", () => {
  const originalLang = document.documentElement.lang;
  const originalDir = document.documentElement.dir;

  beforeEach(() => {
    Object.defineProperty(navigator, "languages", { configurable: true, value: ["de-DE", "en"] });
  });

  afterEach(() => {
    document.documentElement.lang = originalLang;
    document.documentElement.dir = originalDir;
  });

  it("formats ICU interpolation and plurals and supports locale-aware helpers", () => {
    const service = new LocalizationService();
    service.activate("en");
    expect(service.format("localization.welcome", { name: "Ada" })).toBe("Welcome, Ada");
    expect(service.format("localization.items", { count: 0 })).toBe("No items");
    expect(service.format("localization.items", { count: 1 })).toBe("1 item");
    expect(service.format("localization.items", { count: 4 })).toBe("4 items");
    expect(service.formatNumber(1234.5)).toBe(new Intl.NumberFormat("en").format(1234.5));
    expect(service.formatRelativeTime(-3, "minute")).toContain("3");
    expect(service.formatDate(new Date(2026, 8, 6))).not.toBe("");
    expect(service.formatTime(new Date(2026, 8, 6, 14, 30))).not.toBe("");
  });

  it("negotiates OS and explicit locales, sets direction, and falls back per key", () => {
    const service = new LocalizationService();
    expect(service.activate("auto").locale).toBe("de");
    expect(service.format("localization.catalogError", { locale: "de" })).toContain("English");
    expect(service.activate("ar-EG").direction).toBe("rtl");
    expect(document.documentElement).toHaveAttribute("lang", "ar");
    expect(document.documentElement).toHaveAttribute("dir", "rtl");
    const unsupported = service.activate("xx-ZZ");
    expect(unsupported.locale).toBe("en");
    expect(unsupported.notice).toContain("xx-ZZ");
    expect(service.activate("xx-ZZ").notice).toBeNull();
    expect(service.format("does.not.exist")).toBe("Missing translation");
    expect(service.formatMessage({ id: "also.missing" })).toBe("Missing translation");
    expect(negotiateLocale(["fr-CA"], service.availableLocales())).toBe("fr");
    expect(localeDirection("he-IL")).toBe("rtl");
  });

  it("pseudo-localizes visible text while preserving ICU arguments", () => {
    const service = new LocalizationService();
    service.activate(PSEUDO_LOCALE);
    expect(service.format("localization.welcome", { name: "Ada" })).toMatch(/^［.+Ada］$/);
    expect(service.format("localization.items", { count: 2 })).toContain("2");
  });

  it("loads plugin catalogs with locale fallback and recovers from malformed catalogs", () => {
    const service = new LocalizationService();
    expect(
      service.registerCatalog(
        "en",
        { "plugin.greeting": "Plugin hello, {name}" },
        "plugin.example",
      ),
    ).toBe(true);
    expect(
      service.registerCatalog(
        "de",
        { "plugin.greeting": "Plugin-Hallo, {name}" },
        "plugin.example",
      ),
    ).toBe(true);
    service.activate("de");
    expect(service.format("plugin.greeting", { name: "Ada" })).toBe("Plugin-Hallo, Ada");
    expect(service.registerCatalog("fr", "{", "broken.plugin")).toBe(false);
    expect(service.current.locale).toBe("en");
    expect(service.current.notice).toContain("fr");
    service.unregisterCatalogs("plugin.example");
    expect(service.format("plugin.greeting", { name: "Ada" })).toBe("Missing translation");
  });
});
