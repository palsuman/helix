import { beforeEach, describe, expect, it } from "vitest";
import {
  EDITOR_TABS_STORAGE_KEY,
  closeEditorTab,
  closeAllEditorTabs,
  closeEditorTabsToRight,
  closeOtherEditorTabs,
  createEditorTab,
  openEditorTab,
  parseEditorTabs,
  promoteEditorTab,
  reorderEditorTabs,
  setEditorTabDirty,
  toggleEditorTabPinned,
  type EditorTabState,
} from "./tabs";

const state = (): EditorTabState => ({
  tabs: [createEditorTab("/a.ts"), createEditorTab("/b.ts"), createEditorTab("/c.ts")],
  activeId: "/a.ts",
});

describe("editor tab state", () => {
  beforeEach(() => localStorage.removeItem(EDITOR_TABS_STORAGE_KEY));

  it("reuses a path and replaces only a clean preview tab", () => {
    const first = openEditorTab({ tabs: [], activeId: null }, "/a.ts");
    const second = openEditorTab(first, "/b.ts");
    expect(second.tabs.map((tab) => tab.path)).toEqual(["/b.ts"]);
    expect(openEditorTab(second, "/b.ts").tabs).toHaveLength(1);
    expect(openEditorTab(setEditorTabDirty(second, "/b.ts", true), "/c.ts").tabs).toHaveLength(2);
  });

  it("promotes, pins, reorders, and closes tabs", () => {
    let current = toggleEditorTabPinned(promoteEditorTab(state(), "/a.ts"), "/a.ts");
    current = reorderEditorTabs(current, "/c.ts", "/b.ts");
    expect(current.tabs.map((tab) => tab.path)).toEqual(["/a.ts", "/c.ts", "/b.ts"]);
    expect(closeEditorTab(current, "/a.ts").activeId).toBe("/c.ts");
  });

  it("supports close others and close to the right", () => {
    const current = toggleEditorTabPinned(state(), "/a.ts");
    expect(closeOtherEditorTabs(current, "/c.ts").tabs.map((tab) => tab.path)).toEqual(["/a.ts", "/c.ts"]);
    expect(closeEditorTabsToRight(current, "/b.ts").tabs.map((tab) => tab.path)).toEqual(["/a.ts", "/b.ts"]);
  });

  it("supports close all", () => {
    expect(closeAllEditorTabs()).toEqual({ tabs: [], activeId: null });
  });

  it("rejects corrupt persistence and repairs an invalid active id", () => {
    expect(parseEditorTabs({ tabs: [{ path: "/a.ts" }] })).toBeNull();
    const parsed = parseEditorTabs({ tabs: [createEditorTab("/a.ts")], activeId: "/missing" });
    expect(parsed?.activeId).toBe("/a.ts");
  });
});