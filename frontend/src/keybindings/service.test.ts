import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextKeyService } from "./context";
import { keybindingHarness } from "./testUtils";

const press = (key: string, modifiers: KeyboardEventInit = {}) =>
  new KeyboardEvent("keydown", { key, cancelable: true, ...modifiers });

describe("keybinding service", () => {
  afterEach(() => vi.useRealTimers());

  it("dispatches only the winning registered command over IPC", async () => {
    const kernel = keybindingHarness();
    await kernel.service.refresh();
    await kernel.service.contribute("example", [
      { key: "meta+k z", command: "example.run", args: { mode: "fast" } },
    ]);
    kernel.service.handle(press("k", { metaKey: true }));
    const final = press("z");
    expect(kernel.service.handle(final)).toBe(true);
    expect(final.defaultPrevented).toBe(true);
    await Promise.resolve();
    await Promise.resolve();
    await vi.waitFor(() =>
      expect(kernel.executed).toEqual([{ command: "example.run", args: { mode: "fast" } }]),
    );
    expect(kernel.requests.filter((request) => request.command === "command.execute")).toHaveLength(
      1,
    );
    await kernel.service.removeContribution("example");
    expect(kernel.service.shortcutFor("example.run")).toBeNull();
    expect(kernel.service.shortcutFor("workbench.action.toggleZenMode")).toBe("meta+k z");
  });

  it("saves a rebind, supports removal and reset, and reloads external changes", async () => {
    const kernel = keybindingHarness();
    await kernel.service.refresh();
    await kernel.service.rebind("editor.action.formatDocument", "meta+alt+l", "editorTextFocus");
    expect(kernel.document().user).toContainEqual({
      key: "alt+shift+f",
      command: "-editor.action.formatDocument",
      when: "editorTextFocus && !editorReadonly",
    });
    expect(kernel.service.shortcutFor("editor.action.formatDocument")).toBe("alt+meta+l");
    kernel.externalEdit([{ key: "meta+r", command: "editor.action.formatDocument" }]);
    await expect(kernel.service.save([])).rejects.toThrow("changed on disk");
    await kernel.service.refresh();
    expect(kernel.service.shortcutFor("editor.action.formatDocument")).toBe("meta+r");
    await kernel.service.resetCommand("editor.action.formatDocument");
    expect(kernel.service.shortcutFor("editor.action.formatDocument")).toBe("alt+shift+f");
    const binding = kernel.service.store
      .getState()
      .bindings.find((entry) => entry.command === "editor.action.formatDocument")!;
    await kernel.service.remove(binding);
    expect(kernel.service.shortcutFor("editor.action.formatDocument")).toBeNull();
  });

  it("polls changes across windows and cancels pending chords and subscriptions on stop", async () => {
    vi.useFakeTimers();
    const kernel = keybindingHarness();
    const stop = kernel.service.start();
    await vi.advanceTimersByTimeAsync(0);
    kernel.externalEdit([{ key: "meta+y", command: "example.run" }]);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(kernel.service.shortcutFor("example.run")).toBe("meta+y");
    window.dispatchEvent(press("k", { metaKey: true }));
    expect(kernel.service.store.getState().pending).toEqual(["meta+k"]);
    window.dispatchEvent(new Event("blur"));
    expect(kernel.service.store.getState().pending).toEqual([]);
    stop();
    expect(vi.getTimerCount()).toBe(0);
    const event = press("y", { metaKey: true });
    window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it("derives focus contexts without treating shell placeholders as editors", () => {
    const context = new ContextKeyService();
    const editor = document.createElement("div");
    editor.dataset.keybindingContext = "editor";
    editor.dataset.languageId = "typescript";
    const input = document.createElement("textarea");
    editor.append(input);
    context.focus(input);
    expect(context.store.getState()).toMatchObject({
      editorTextFocus: true,
      editorLangId: "typescript",
      inputFocus: true,
    });
    const search = document.createElement("input");
    search.type = "search";
    context.focus(search);
    expect(context.store.getState()).toMatchObject({ editorTextFocus: false, inSearch: true });
    context.set({ debugActive: true, workspaceOpen: true });
    context.focus(null);
    expect(context.store.getState()).toMatchObject({
      debugActive: true,
      workspaceOpen: true,
      inSearch: false,
    });
  });

  it("does not steal ordinary input or consume bindings whose when-clause is malformed", async () => {
    const kernel = keybindingHarness("linux", [
      { key: "h", command: "cursorLeft", when: "editorTextFocus && vimMode == 'normal'" },
      { key: "ctrl+y", command: "example.run", when: "editorTextFocus &&" },
    ]);
    await kernel.service.refresh();
    const stop = kernel.service.start();
    try {
      const input = document.createElement("input");
      document.body.append(input);
      const event = new KeyboardEvent("keydown", { key: "h", bubbles: true, cancelable: true });
      input.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
      expect(kernel.service.store.getState().warnings).toHaveLength(1);
      expect(kernel.service.shortcutFor("example.run")).toBeNull();
      input.remove();
    } finally {
      stop();
    }
  });
});
