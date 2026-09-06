import { createIntl, createIntlCache, type IntlShape, type MessageDescriptor } from "react-intl";
import {
  createLiteralElement,
  isLiteralElement,
  isPluralElement,
  isSelectElement,
  isTagElement,
  parse,
  type MessageFormatElement,
} from "@formatjs/icu-messageformat-parser";
import ar from "./locales/ar.json";
import de from "./locales/de.json";
import en from "./locales/en.json";
import es from "./locales/es.json";
import fr from "./locales/fr.json";
import ja from "./locales/ja.json";

export const BASE_LOCALE = "en";
export const PSEUDO_LOCALE = "en-XA";
export const LOCALE_SETTING = "helix.locale";
export type TextDirection = "ltr" | "rtl";
export type Catalog = Record<string, string | MessageFormatElement[]>;
type IntlCatalog = Record<string, string> | Record<string, MessageFormatElement[]>;

const BUNDLED_CATALOGS: Record<string, Catalog> = { en, de, es, fr, ja, ar };
const RTL_LANGUAGES = new Set(["ar", "fa", "he", "ps", "ur"]);
const pseudoMap: Record<string, string> = {
  a: "à",
  b: "ƀ",
  c: "ç",
  d: "ð",
  e: "ë",
  f: "ƒ",
  g: "ğ",
  h: "ħ",
  i: "ï",
  j: "ĵ",
  k: "ķ",
  l: "ļ",
  m: "ɱ",
  n: "ñ",
  o: "ö",
  p: "þ",
  q: "ǫ",
  r: "ř",
  s: "š",
  t: "ŧ",
  u: "ü",
  v: "ṽ",
  w: "ŵ",
  x: "ẋ",
  y: "ÿ",
  z: "ž",
};

function pseudoLiteral(message: string): string {
  let result = "";
  for (const character of message) {
    if (/[a-z]/i.test(character)) {
      const mapped = pseudoMap[character.toLowerCase()] ?? character;
      result += character === character.toUpperCase() ? mapped.toUpperCase() : mapped;
      if (/[aeiou]/i.test(character)) result += mapped;
    } else {
      result += character;
    }
  }
  return result;
}

export function pseudoCatalog(catalog: Catalog): Catalog {
  return Object.fromEntries(
    Object.entries(catalog).map(([id, message]) => {
      const ast = typeof message === "string" ? parse(message) : message;
      return [id, [createLiteralElement("［"), ...pseudoElements(ast), createLiteralElement("］")]];
    }),
  );
}

function pseudoElements(elements: readonly MessageFormatElement[]): MessageFormatElement[] {
  return elements.map((element) => {
    if (isLiteralElement(element)) return { ...element, value: pseudoLiteral(element.value) };
    if (isPluralElement(element) || isSelectElement(element)) {
      return {
        ...element,
        options: Object.fromEntries(
          Object.entries(element.options).map(([selector, option]) => [
            selector,
            { ...option, value: pseudoElements(option.value) },
          ]),
        ),
      };
    }
    if (isTagElement(element)) {
      return { ...element, children: pseudoElements(element.children) };
    }
    return { ...element };
  });
}

export function localeDirection(locale: string): TextDirection {
  return RTL_LANGUAGES.has(locale.split("-")[0]!.toLowerCase()) ? "rtl" : "ltr";
}

export function negotiateLocale(
  requested: readonly string[],
  available: readonly string[],
): string {
  const normalized = new Map(available.map((locale) => [locale.toLowerCase(), locale]));
  for (const request of requested) {
    const exact = normalized.get(request.toLowerCase());
    if (exact) return exact;
    const language = request.split("-")[0]?.toLowerCase();
    if (language && normalized.has(language)) return normalized.get(language)!;
  }
  return BASE_LOCALE;
}

export interface LocalizationSnapshot {
  locale: string;
  direction: TextDirection;
  intl: IntlShape;
  notice: string | null;
}

export class LocalizationService {
  private readonly catalogs = new Map<string, Catalog>();
  private readonly pluginCatalogs = new Map<string, Map<string, Catalog>>();
  private readonly listeners = new Set<() => void>();
  private readonly cache = createIntlCache();
  private readonly reportedUnsupportedLocales = new Set<string>();
  private resolvedMessages: IntlCatalog = {};
  private snapshot: LocalizationSnapshot;

  constructor(catalogs: Record<string, Catalog> = BUNDLED_CATALOGS) {
    for (const [locale, catalog] of Object.entries(catalogs)) this.catalogs.set(locale, catalog);
    if (!this.catalogs.has(BASE_LOCALE)) throw new Error("The English base catalog is required.");
    this.catalogs.set(PSEUDO_LOCALE, pseudoCatalog(this.catalogs.get(BASE_LOCALE)!));
    this.snapshot = this.createSnapshot(BASE_LOCALE, null);
  }

  get current(): LocalizationSnapshot {
    return this.snapshot;
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private emit(): void {
    for (const listener of this.listeners) listener();
  }

  availableLocales(): string[] {
    return [...this.catalogs.keys()];
  }

  activate(
    requested: string | null | undefined,
    osLocales = navigator.languages,
  ): LocalizationSnapshot {
    const explicit = requested?.trim() && requested !== "auto" ? [requested] : [];
    const requestedLocales = explicit.length ? explicit : osLocales;
    const locale = negotiateLocale(requestedLocales, this.availableLocales());
    const unsupported =
      explicit.length > 0 && locale === BASE_LOCALE && explicit[0]?.toLowerCase() !== BASE_LOCALE;
    const unsupportedLocale = explicit[0];
    const shouldReportUnsupported =
      unsupported &&
      unsupportedLocale !== undefined &&
      !this.reportedUnsupportedLocales.has(unsupportedLocale.toLowerCase());
    if (shouldReportUnsupported) {
      this.reportedUnsupportedLocales.add(unsupportedLocale!.toLowerCase());
    }
    this.snapshot = this.createSnapshot(
      locale,
      shouldReportUnsupported
        ? this.formatBase("localization.unsupportedLocale", { locale: unsupportedLocale! })
        : null,
    );
    this.applyDocumentLanguage();
    this.emit();
    return this.snapshot;
  }

  registerCatalog(locale: string, catalog: string | Catalog, owner = "application"): boolean {
    try {
      const parsed = typeof catalog === "string" ? (JSON.parse(catalog) as unknown) : catalog;
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed))
        throw new Error("Catalog must be an object.");
      const messages: Catalog = {};
      for (const [id, value] of Object.entries(parsed)) {
        if (typeof value !== "string" || value.trim() === "")
          throw new Error(`Message '${id}' must be a non-empty string.`);
        messages[id] = value;
      }
      const owners = this.pluginCatalogs.get(owner) ?? new Map<string, Catalog>();
      owners.set(locale, messages);
      this.pluginCatalogs.set(owner, owners);
      if (locale === this.snapshot.locale) {
        this.snapshot = this.createSnapshot(locale, null);
        this.emit();
      }
      return true;
    } catch {
      this.snapshot = this.createSnapshot(
        BASE_LOCALE,
        this.formatBase("localization.catalogError", { locale }),
      );
      this.applyDocumentLanguage();
      this.emit();
      return false;
    }
  }

  unregisterCatalogs(owner: string): void {
    this.pluginCatalogs.delete(owner);
    this.snapshot = this.createSnapshot(this.snapshot.locale, null);
    this.emit();
  }

  format(id: string, values?: Record<string, string | number | Date>): string {
    const fallback = this.baseMessage(id) ?? this.resolvedMessages[id];
    if (!fallback) return this.baseMessage("localization.missing") ?? "Missing translation";
    return this.snapshot.intl.formatMessage({ id, defaultMessage: fallback }, values);
  }

  formatMessage(
    descriptor: MessageDescriptor,
    values?: Record<string, string | number | Date>,
  ): string {
    const id = String(descriptor.id ?? "");
    const fallback = this.baseMessage(id) ?? descriptor.defaultMessage;
    if (!fallback) return this.baseMessage("localization.missing") ?? "Missing translation";
    return this.snapshot.intl.formatMessage(
      { ...descriptor, id, defaultMessage: fallback },
      values,
    );
  }

  formatDate(value: Date | number, options?: Intl.DateTimeFormatOptions): string {
    return this.snapshot.intl.formatDate(value, options);
  }

  formatTime(value: Date | number, options?: Intl.DateTimeFormatOptions): string {
    return this.snapshot.intl.formatTime(value, options);
  }

  formatNumber(value: number, options?: Intl.NumberFormatOptions): string {
    return this.snapshot.intl.formatNumber(value, options);
  }

  formatRelativeTime(value: number, unit: Intl.RelativeTimeFormatUnit = "second"): string {
    return this.snapshot.intl.formatRelativeTime(value, unit, { numeric: "auto" });
  }

  private messages(locale: string): Catalog {
    const merged = {
      ...(this.catalogs.get(BASE_LOCALE) ?? {}),
      ...(this.catalogs.get(locale) ?? {}),
    };
    for (const catalogs of this.pluginCatalogs.values()) {
      Object.assign(merged, catalogs.get(BASE_LOCALE) ?? {}, catalogs.get(locale) ?? {});
    }
    return merged;
  }

  private intlMessages(locale: string): IntlCatalog {
    const messages = this.messages(locale);
    if (Object.values(messages).some(Array.isArray)) {
      return Object.fromEntries(
        Object.entries(messages).map(([id, message]) => [
          id,
          typeof message === "string" ? parse(message) : message,
        ]),
      );
    }
    return messages as Record<string, string>;
  }

  private baseMessage(id: string): string | undefined {
    const message = this.catalogs.get(BASE_LOCALE)?.[id];
    return typeof message === "string" ? message : undefined;
  }

  private createSnapshot(locale: string, notice: string | null): LocalizationSnapshot {
    const direction = localeDirection(locale);
    this.resolvedMessages = this.intlMessages(locale);
    const intl = createIntl(
      {
        locale,
        defaultLocale: BASE_LOCALE,
        messages: this.resolvedMessages,
        onError: () => {},
      },
      this.cache,
    );
    return { locale, direction, intl, notice };
  }

  private formatBase(id: string, values?: Record<string, string | number>): string {
    const message = this.baseMessage(id) ?? "Localization error";
    const intl = createIntl(
      { locale: BASE_LOCALE, messages: this.intlMessages(BASE_LOCALE) },
      this.cache,
    );
    return intl.formatMessage({ id, defaultMessage: message }, values);
  }

  private applyDocumentLanguage(): void {
    document.documentElement.lang = this.snapshot.locale;
    document.documentElement.dir = this.snapshot.direction;
  }
}

export const localization = new LocalizationService();
