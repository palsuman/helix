import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CHORD_TIMEOUT_MS,
  eventChord,
  findConflicts,
  KeybindingResolver,
  normalizeKeybinding,
  resolveBindings,
} from "./resolver";

const key = (value: string, options: KeyboardEventInit = {}) =>
  new KeyboardEvent("keydown", { key: value, ...options });

describe("keybinding resolver", () => {
  afterEach(() => vi.useRealTimers());

  it("normalizes platform modifiers, chords, plus and space", () => {
    expect(normalizeKeybinding("Cmd+Shift+P")).toEqual(["shift+meta+p"]);
    expect(normalizeKeybinding("Ctrl+K Ctrl+C")).toEqual(["ctrl+k", "ctrl+c"]);
    expect(normalizeKeybinding("Ctrl++")).toEqual(["ctrl+plus"]);
    expect(normalizeKeybinding("Ctrl+Ctrl+K")).toBeNull();
    expect(eventChord(key(" "))).toBe("space");
    expect(eventChord(key("Dead"))).toBeNull();
    expect(eventChord(key("k", { isComposing: true }))).toBeNull();
    expect(eventChord(key("$", { code: "Digit4", shiftKey: true }))).toBe("shift+4");
    expect(eventChord(key("¬", { code: "KeyL", altKey: true, metaKey: true }))).toBe("alt+meta+l");
  });

  it("applies user > plugin > default precedence and last-wins ordering with removals", () => {
    const { bindings, warnings } = resolveBindings(
      [
        {
          source: "user",
          owner: "User",
          rules: [
            { key: "ctrl+k", command: "user.first" },
            { key: "ctrl+k", command: "user.last" },
            { key: "ctrl+p", command: "-palette" },
          ],
        },
        {
          source: "default",
          owner: "Helix",
          rules: [
            { key: "ctrl+k", command: "default" },
            { key: "ctrl+p", command: "palette" },
          ],
        },
        {
          source: "plugin",
          owner: "plugin",
          rules: [
            { key: "ctrl+k", command: "plugin" },
            { key: "bad+key", command: "invalid" },
          ],
        },
      ],
      "windows",
    );
    const resolver = new KeybindingResolver();
    expect(resolver.handle(key("k", { ctrlKey: true }), bindings, {}).binding?.command).toBe(
      "user.last",
    );
    expect(resolver.handle(key("p", { ctrlKey: true }), bindings, {}).consumed).toBe(false);
    expect(warnings).toHaveLength(1);
    expect(findConflicts(bindings).size).toBe(4);
  });

  it("resolves platform overrides and current when-clauses", () => {
    const rules = [
      {
        key: "ctrl+p",
        mac: "cmd+p",
        linux: "alt+p",
        command: "run",
        when: "editorTextFocus && !terminalFocus",
      },
    ];
    for (const [platform, shortcut] of [
      ["mac", "meta+p"],
      ["windows", "ctrl+p"],
      ["linux", "alt+p"],
    ] as const) {
      expect(
        resolveBindings([{ source: "default", owner: "Helix", rules }], platform).bindings[0].key,
      ).toBe(shortcut);
    }
    const { bindings } = resolveBindings([{ source: "default", owner: "Helix", rules }], "windows");
    const resolver = new KeybindingResolver();
    expect(
      resolver.handle(key("p", { ctrlKey: true }), bindings, { editorTextFocus: false }).consumed,
    ).toBe(false);
    expect(
      resolver.handle(key("p", { ctrlKey: true }), bindings, { editorTextFocus: true }).binding
        ?.command,
    ).toBe("run");
    expect(
      resolver.handle(key("p", { ctrlKey: true }), bindings, { editorTextFocus: true }, () => false)
        .consumed,
    ).toBe(false);
  });

  it("completes chords within 1.5s, cancels on Escape, and releases stale prefixes", () => {
    vi.useFakeTimers();
    const { bindings } = resolveBindings(
      [{ source: "default", owner: "Helix", rules: [{ key: "ctrl+k z", command: "zen" }] }],
      "linux",
    );
    const pending = vi.fn();
    const resolver = new KeybindingResolver(pending);
    expect(resolver.handle(key("k", { ctrlKey: true }), bindings, {}).consumed).toBe(true);
    vi.advanceTimersByTime(CHORD_TIMEOUT_MS - 1);
    expect(resolver.handle(key("z"), bindings, {}).binding?.command).toBe("zen");
    resolver.handle(key("k", { ctrlKey: true }), bindings, {});
    vi.advanceTimersByTime(CHORD_TIMEOUT_MS);
    expect(pending).toHaveBeenLastCalledWith([]);
    expect(resolver.handle(key("z"), bindings, {}).consumed).toBe(false);
    resolver.handle(key("k", { ctrlKey: true }), bindings, {});
    resolver.handle(key("Escape"), bindings, {});
    expect(resolver.handle(key("z"), bindings, {}).consumed).toBe(false);
    resolver.handle(key("k", { ctrlKey: true }), bindings, {});
    resolver.handle(key("Process", { isComposing: true }), bindings, {});
    expect(resolver.handle(key("z"), bindings, {}).consumed).toBe(false);
    resolver.reset();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("flags prefix conflicts but not mutually exclusive simple contexts", () => {
    const { bindings } = resolveBindings(
      [
        {
          source: "default",
          owner: "Helix",
          rules: [
            { key: "ctrl+k", command: "first", when: "editorTextFocus" },
            { key: "ctrl+k", command: "second", when: "!editorTextFocus" },
            { key: "ctrl+k z", command: "third", when: "editorTextFocus" },
          ],
        },
      ],
      "linux",
    );
    const conflicts = findConflicts(bindings);
    expect(conflicts.get(bindings[0].id)?.map((rule) => rule.command)).toEqual(["third"]);
    expect(conflicts.has(bindings[1].id)).toBe(false);
  });
});
