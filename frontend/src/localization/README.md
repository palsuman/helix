# Localization

Helix resolves UI text through `LocalizationProvider` and FormatJS. The base
catalog is English; German, Spanish, French, Japanese, and Arabic catalogs are
shipped and fall back to English per key. `en-XA` is the expansion pseudo-locale.

The provider reads the user-scoped `helix.locale` setting. `auto` negotiates
`navigator.languages`, first by exact locale and then language. Unsupported
explicit locales fall back to English and produce one notice per requested
locale. Locale changes do not require reinstalling; because the setting is
restart-required, changes from another window produce a restart notification.
The service can also activate a locale live for previews and tests.

## Adding Messages

1. Add a descriptor to `messages.ts` with a stable ID and English
   `defaultMessage`. Use ICU arguments, plurals, and selects for grammatical
   construction.
2. Render it with `useMessage()` in React or `message()` in a non-component
   producer.
3. Run `npm run i18n:extract` from `frontend/` to regenerate `locales/en.json`.
4. Add translations to locale JSON files. Missing keys automatically use the
   English message.

`npm run i18n:check` rejects base-catalog drift. The ESLint
`helix/no-user-visible-literals` rule rejects visible JSX text and literal
accessible labels, including text inside templates and conditionals. Tests and
the descriptor registry are exempt. Both gates run in the production build;
the rule's `RuleTester` suite also runs before Vitest.

`LocalizationService` exposes date, time, number, and relative-time formatters.
It accepts plugin catalogs through `registerCatalog(locale, catalog, owner)` and
removes them with `unregisterCatalogs(owner)`. Plugin command descriptors may
provide title, category, and disabled-reason message IDs; metadata always keeps
its contributed fallback text when no catalog entry exists. Malformed runtime
catalogs fall back to English without blocking startup.

## RTL And Unicode

Activating a locale updates the root `lang` and `dir` attributes. Layout CSS uses
logical properties, so flex ordering and offsets mirror automatically. SVG
assets marked `data-rtl-mirror="true"` are emitted in the generated mirror list;
`Icon` applies the RTL transform without consumer-specific classes. Dynamic
paths, command titles, notification text, and log text use bidirectional
isolation or automatic direction.

Editor text behavior remains independent of UI locale. `helix_core::text`
provides UTF-8 byte-based previous/next grapheme boundaries, grapheme deletion,
CJK-aware display width, and warning spans for bidi controls, suspicious
invisibles, and common Cyrillic/Greek confusables. Warning decorations expose
stable class names and hover text. A future editor adapter converts byte ranges
to its native position model and applies the provided decoration classes;
joiners needed by emoji and shaping scripts are not blanket-flagged.
