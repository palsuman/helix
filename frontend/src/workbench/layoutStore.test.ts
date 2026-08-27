import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_LAYOUT,
  layoutSnapshot,
  listLayoutProfiles,
  parseLayout,
  useLayoutStore,
} from "./layoutStore";

describe("layout profiles", () => {
  beforeEach(() => useLayoutStore.getState().reset());

  it("round-trips the exact Debug layout after switching through Writing", () => {
    const store = useLayoutStore.getState();
    store.setPrimarySidebarSize(340);
    store.setPanelPosition("right");
    store.setPanelSize(310);
    store.splitEditor("vertical");
    store.saveProfile("Debug");
    const debugGeometry = layoutSnapshot().profiles[0]?.layout;
    expect(debugGeometry).toBeDefined();

    store.setPrimarySidebarSize(220);
    store.setPanelPosition("bottom");
    store.setPanelSize(160);
    store.setSecondarySidebarVisible(false);
    useLayoutStore.getState().saveProfile("Writing");

    expect(useLayoutStore.getState().switchProfile("Debug")).toEqual([]);
    expect(layoutSnapshot()).toMatchObject(debugGeometry!);
    expect(listLayoutProfiles()).toEqual([
      { name: "Debug", active: true },
      { name: "Writing", active: false },
    ]);
  });

  it("switches, renames, and deletes profiles without losing other snapshots", () => {
    const store = useLayoutStore.getState();
    store.saveProfile("Debug");
    store.setPanelVisible(false);
    useLayoutStore.getState().saveProfile("Writing");

    useLayoutStore.getState().renameProfile("Writing", "Focus");
    expect(listLayoutProfiles()).toEqual([
      { name: "Debug", active: false },
      { name: "Focus", active: true },
    ]);
    useLayoutStore.getState().deleteProfile("Debug");
    expect(listLayoutProfiles()).toEqual([{ name: "Focus", active: true }]);
  });

  it("loads a profile with missing views as empty slots and reports them once", () => {
    const store = useLayoutStore.getState();
    store.setActiveActivity("removed.activity");
    store.setActivePanel("removed.panel");
    store.saveProfile("Old plugin");

    const missing = useLayoutStore.getState().switchProfile("Old plugin", {
      activityIds: ["explorer"],
      panelIds: ["problems"],
    });

    expect(missing).toEqual(["removed.activity", "removed.panel"]);
    expect(useLayoutStore.getState().activeActivity).toBe("");
    expect(useLayoutStore.getState().activePanel).toBe("");
    expect(useLayoutStore.getState().profileNotice).toContain("removed.activity");
    useLayoutStore.getState().dismissProfileNotice();
    expect(useLayoutStore.getState().profileNotice).toBeNull();
  });

  it("rejects a corrupt profile store so persistence can reset to defaults", () => {
    expect(
      parseLayout({
        ...DEFAULT_LAYOUT,
        profiles: [{ name: "Broken", layout: { panelSize: "huge" } }],
      }),
    ).toBeNull();
  });

  it("rejects non-finite dimensions and duplicate editor groups as corrupt geometry", () => {
    expect(parseLayout({ ...DEFAULT_LAYOUT, panelSize: Number.POSITIVE_INFINITY })).toBeNull();
    expect(
      parseLayout({
        ...DEFAULT_LAYOUT,
        editorGroups: ["editor-1", "editor-1"],
      }),
    ).toBeNull();
    expect(
      parseLayout({
        ...DEFAULT_LAYOUT,
        profiles: [
          {
            name: "Broken",
            layout: {
              ...DEFAULT_LAYOUT,
              editorGroups: ["editor-1", "editor-1"],
            },
          },
        ],
      }),
    ).toBeNull();
  });

  it("persists profiles but not transient zen or notice state", () => {
    useLayoutStore.getState().saveProfile("Default");
    useLayoutStore.getState().toggleZenMode();
    const snapshot = layoutSnapshot();

    expect(snapshot.profiles).toHaveLength(1);
    expect(snapshot.activeProfile).toBe("Default");
    expect(snapshot).not.toHaveProperty("zenMode");
    expect(snapshot).not.toHaveProperty("profileNotice");
    expect(useLayoutStore.getState().zenMode).toBe(true);
    useLayoutStore.getState().toggleZenMode();
    expect(useLayoutStore.getState().zenMode).toBe(false);
    expect(useLayoutStore.getState().primarySidebarVisible).toBe(true);
  });

  it("allocates a unique editor group after restoring profile geometry", () => {
    useLayoutStore.getState().restore({
      ...DEFAULT_LAYOUT,
      editorGroups: ["editor-1", "editor-2"],
      activeEditorGroup: "editor-1",
    });

    useLayoutStore.getState().splitEditor("horizontal");

    const groups = useLayoutStore.getState().editorGroups;
    expect(groups).toHaveLength(3);
    expect(new Set(groups).size).toBe(3);
  });
});
