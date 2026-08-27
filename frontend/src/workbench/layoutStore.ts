import { create } from "zustand";

export const PRIMARY_SIDEBAR_MIN = 200;
export const PRIMARY_SIDEBAR_MAX = 600;
export const SECONDARY_SIDEBAR_MIN = 200;
export const SECONDARY_SIDEBAR_MAX = 500;
export const PANEL_MIN = 120;
export const PANEL_MAX = 500;

export type Side = "left" | "right";
export type PanelPosition = "bottom" | "right";
export type SplitDirection = "horizontal" | "vertical";

export interface LayoutGeometry {
  primarySidebarVisible: boolean;
  primarySidebarPosition: Side;
  primarySidebarSize: number;
  secondarySidebarVisible: boolean;
  secondarySidebarSize: number;
  panelVisible: boolean;
  panelPosition: PanelPosition;
  panelSize: number;
  activeActivity: string;
  activePanel: string;
  splitDirection: SplitDirection;
  editorGroups: string[];
  activeEditorGroup: string;
}

export interface LayoutProfile {
  name: string;
  layout: LayoutGeometry;
}

export interface WorkbenchLayout extends LayoutGeometry {
  profiles: LayoutProfile[];
  activeProfile: string | null;
}

export interface ProfileViewAvailability {
  activityIds: readonly string[];
  panelIds: readonly string[];
}

interface LayoutActions {
  setPrimarySidebarVisible(visible: boolean): void;
  setPrimarySidebarPosition(position: Side): void;
  setPrimarySidebarSize(size: number): void;
  setSecondarySidebarVisible(visible: boolean): void;
  setSecondarySidebarSize(size: number): void;
  setPanelVisible(visible: boolean): void;
  setPanelPosition(position: PanelPosition): void;
  setPanelSize(size: number): void;
  setActiveActivity(id: string): void;
  setActivePanel(id: string): void;
  setActiveEditorGroup(id: string): void;
  splitEditor(direction: SplitDirection): void;
  closeEditorGroup(id: string): void;
  saveProfile(name: string): void;
  switchProfile(name: string, available?: ProfileViewAvailability): string[];
  reconcileActiveProfileViews(available: ProfileViewAvailability): string[];
  renameProfile(currentName: string, nextName: string): void;
  deleteProfile(name: string): void;
  toggleZenMode(): void;
  dismissProfileNotice(): void;
  restore(layout: WorkbenchLayout): void;
  reset(): void;
}

interface TransientLayoutState {
  zenMode: boolean;
  profileNotice: string | null;
}

export type LayoutStore = WorkbenchLayout & TransientLayoutState & LayoutActions;

export const DEFAULT_LAYOUT: WorkbenchLayout = {
  primarySidebarVisible: true,
  primarySidebarPosition: "left",
  primarySidebarSize: 260,
  secondarySidebarVisible: true,
  secondarySidebarSize: 240,
  panelVisible: true,
  panelPosition: "bottom",
  panelSize: 220,
  activeActivity: "explorer",
  activePanel: "problems",
  splitDirection: "horizontal",
  editorGroups: ["editor-1"],
  activeEditorGroup: "editor-1",
  profiles: [],
  activeProfile: null,
};

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, Math.round(value)));

const cleanProfileName = (name: string) => {
  const normalized = name.trim();
  if (normalized.length === 0) throw new Error("A layout profile name cannot be empty.");
  return normalized;
};

function geometrySnapshot(state: LayoutGeometry): LayoutGeometry {
  return {
    primarySidebarVisible: state.primarySidebarVisible,
    primarySidebarPosition: state.primarySidebarPosition,
    primarySidebarSize: state.primarySidebarSize,
    secondarySidebarVisible: state.secondarySidebarVisible,
    secondarySidebarSize: state.secondarySidebarSize,
    panelVisible: state.panelVisible,
    panelPosition: state.panelPosition,
    panelSize: state.panelSize,
    activeActivity: state.activeActivity,
    activePanel: state.activePanel,
    splitDirection: state.splitDirection,
    editorGroups: [...state.editorGroups],
    activeEditorGroup: state.activeEditorGroup,
  };
}

const changed = <T extends Partial<LayoutGeometry>>(value: T) => ({
  ...value,
  activeProfile: null,
});

function removeUnavailableViews(
  layout: LayoutGeometry,
  available: ProfileViewAvailability,
): string[] {
  const missing: string[] = [];
  if (layout.activeActivity && !available.activityIds.includes(layout.activeActivity)) {
    missing.push(layout.activeActivity);
    layout.activeActivity = "";
  }
  if (layout.activePanel && !available.panelIds.includes(layout.activePanel)) {
    missing.push(layout.activePanel);
    layout.activePanel = "";
  }
  return missing;
}

const missingViewsNotice = (name: string, missing: readonly string[]) =>
  `Layout profile '${name}' loaded with unavailable views left empty: ${missing.join(", ")}.`;

let editorSequence = 1;

export const useLayoutStore = create<LayoutStore>((set, get) => ({
  ...DEFAULT_LAYOUT,
  profiles: [],
  zenMode: false,
  profileNotice: null,
  setPrimarySidebarVisible: (primarySidebarVisible) => set(changed({ primarySidebarVisible })),
  setPrimarySidebarPosition: (primarySidebarPosition) => set(changed({ primarySidebarPosition })),
  setPrimarySidebarSize: (primarySidebarSize) =>
    set(
      changed({
        primarySidebarSize: clamp(primarySidebarSize, PRIMARY_SIDEBAR_MIN, PRIMARY_SIDEBAR_MAX),
      }),
    ),
  setSecondarySidebarVisible: (secondarySidebarVisible) =>
    set(changed({ secondarySidebarVisible })),
  setSecondarySidebarSize: (secondarySidebarSize) =>
    set(
      changed({
        secondarySidebarSize: clamp(
          secondarySidebarSize,
          SECONDARY_SIDEBAR_MIN,
          SECONDARY_SIDEBAR_MAX,
        ),
      }),
    ),
  setPanelVisible: (panelVisible) => set(changed({ panelVisible })),
  setPanelPosition: (panelPosition) => set(changed({ panelPosition })),
  setPanelSize: (panelSize) => set(changed({ panelSize: clamp(panelSize, PANEL_MIN, PANEL_MAX) })),
  setActiveActivity: (activeActivity) =>
    set(changed({ activeActivity, primarySidebarVisible: true })),
  setActivePanel: (activePanel) => set(changed({ activePanel, panelVisible: true })),
  setActiveEditorGroup: (activeEditorGroup) => set(changed({ activeEditorGroup })),
  splitEditor: (splitDirection) =>
    set((state) => {
      if (state.editorGroups.length >= 4) return changed({ splitDirection });
      let activeEditorGroup: string;
      do {
        editorSequence += 1;
        activeEditorGroup = `editor-${editorSequence}`;
      } while (state.editorGroups.includes(activeEditorGroup));
      return changed({
        splitDirection,
        activeEditorGroup,
        editorGroups: [...state.editorGroups, activeEditorGroup],
      });
    }),
  closeEditorGroup: (id) =>
    set((state) => {
      if (state.editorGroups.length === 1) return state;
      const editorGroups = state.editorGroups.filter((group) => group !== id);
      return changed({
        editorGroups,
        activeEditorGroup:
          state.activeEditorGroup === id
            ? (editorGroups[0] ?? "editor-1")
            : state.activeEditorGroup,
      });
    }),
  saveProfile: (rawName) => {
    const name = cleanProfileName(rawName);
    set((state) => {
      const profile = { name, layout: geometrySnapshot(state) };
      const existing = state.profiles.findIndex((candidate) => candidate.name === name);
      const profiles = [...state.profiles];
      if (existing === -1) profiles.push(profile);
      else profiles[existing] = profile;
      return { profiles, activeProfile: name, profileNotice: null };
    });
  },
  switchProfile: (rawName, available) => {
    const name = cleanProfileName(rawName);
    const profile = get().profiles.find((candidate) => candidate.name === name);
    if (profile === undefined) throw new Error(`Layout profile '${name}' does not exist.`);
    const layout = geometrySnapshot(profile.layout);
    const missing = available === undefined ? [] : removeUnavailableViews(layout, available);
    set({
      ...layout,
      activeProfile: name,
      profileNotice: missing.length === 0 ? null : missingViewsNotice(name, missing),
    });
    return missing;
  },
  reconcileActiveProfileViews: (available) => {
    const state = get();
    if (state.activeProfile === null) return [];
    const layout = geometrySnapshot(state);
    const missing = removeUnavailableViews(layout, available);
    if (missing.length === 0) return [];
    set({
      activeActivity: layout.activeActivity,
      activePanel: layout.activePanel,
      profileNotice: missingViewsNotice(state.activeProfile, missing),
    });
    return missing;
  },
  renameProfile: (rawCurrentName, rawNextName) => {
    const currentName = cleanProfileName(rawCurrentName);
    const nextName = cleanProfileName(rawNextName);
    set((state) => {
      if (!state.profiles.some((profile) => profile.name === currentName)) {
        throw new Error(`Layout profile '${currentName}' does not exist.`);
      }
      if (currentName !== nextName && state.profiles.some((profile) => profile.name === nextName)) {
        throw new Error(`Layout profile '${nextName}' already exists.`);
      }
      return {
        profiles: state.profiles.map((profile) =>
          profile.name === currentName ? { ...profile, name: nextName } : profile,
        ),
        activeProfile: state.activeProfile === currentName ? nextName : state.activeProfile,
      };
    });
  },
  deleteProfile: (rawName) => {
    const name = cleanProfileName(rawName);
    set((state) => {
      if (!state.profiles.some((profile) => profile.name === name)) {
        throw new Error(`Layout profile '${name}' does not exist.`);
      }
      return {
        profiles: state.profiles.filter((profile) => profile.name !== name),
        activeProfile: state.activeProfile === name ? null : state.activeProfile,
      };
    });
  },
  toggleZenMode: () => set((state) => ({ zenMode: !state.zenMode })),
  dismissProfileNotice: () => set({ profileNotice: null }),
  restore: (layout) =>
    set({ ...layout, profiles: [...layout.profiles], zenMode: false, profileNotice: null }),
  reset: () => set({ ...DEFAULT_LAYOUT, profiles: [], zenMode: false, profileNotice: null }),
}));

export function layoutSnapshot(state: LayoutStore = useLayoutStore.getState()): WorkbenchLayout {
  return {
    ...geometrySnapshot(state),
    profiles: state.profiles.map((profile) => ({
      name: profile.name,
      layout: geometrySnapshot(profile.layout),
    })),
    activeProfile: state.activeProfile,
  };
}

export function listLayoutProfiles(state: LayoutStore = useLayoutStore.getState()) {
  return state.profiles.map((profile) => ({
    name: profile.name,
    active: profile.name === state.activeProfile,
  }));
}

function parseGeometry(value: unknown): LayoutGeometry | null {
  if (typeof value !== "object" || value === null) return null;
  const candidate = value as Partial<LayoutGeometry>;
  if (
    typeof candidate.primarySidebarVisible !== "boolean" ||
    !["left", "right"].includes(candidate.primarySidebarPosition ?? "") ||
    typeof candidate.primarySidebarSize !== "number" ||
    !Number.isFinite(candidate.primarySidebarSize) ||
    typeof candidate.secondarySidebarVisible !== "boolean" ||
    typeof candidate.secondarySidebarSize !== "number" ||
    !Number.isFinite(candidate.secondarySidebarSize) ||
    typeof candidate.panelVisible !== "boolean" ||
    !["bottom", "right"].includes(candidate.panelPosition ?? "") ||
    typeof candidate.panelSize !== "number" ||
    !Number.isFinite(candidate.panelSize) ||
    typeof candidate.activeActivity !== "string" ||
    typeof candidate.activePanel !== "string" ||
    !["horizontal", "vertical"].includes(candidate.splitDirection ?? "") ||
    !Array.isArray(candidate.editorGroups) ||
    candidate.editorGroups.length < 1 ||
    candidate.editorGroups.length > 4 ||
    candidate.editorGroups.some((group) => typeof group !== "string" || group.length === 0) ||
    new Set(candidate.editorGroups).size !== candidate.editorGroups.length ||
    typeof candidate.activeEditorGroup !== "string" ||
    !candidate.editorGroups.includes(candidate.activeEditorGroup)
  ) {
    return null;
  }
  return {
    ...(candidate as LayoutGeometry),
    primarySidebarSize: clamp(
      candidate.primarySidebarSize,
      PRIMARY_SIDEBAR_MIN,
      PRIMARY_SIDEBAR_MAX,
    ),
    secondarySidebarSize: clamp(
      candidate.secondarySidebarSize,
      SECONDARY_SIDEBAR_MIN,
      SECONDARY_SIDEBAR_MAX,
    ),
    panelSize: clamp(candidate.panelSize, PANEL_MIN, PANEL_MAX),
    editorGroups: [...candidate.editorGroups],
  };
}

export function parseLayout(value: unknown): WorkbenchLayout | null {
  const geometry = parseGeometry(value);
  if (geometry === null) return null;
  const candidate = value as Partial<WorkbenchLayout>;
  const rawProfiles = candidate.profiles ?? [];
  if (!Array.isArray(rawProfiles)) return null;
  const profiles: LayoutProfile[] = [];
  const names = new Set<string>();
  for (const rawProfile of rawProfiles) {
    if (typeof rawProfile !== "object" || rawProfile === null) return null;
    const profile = rawProfile as Partial<LayoutProfile>;
    if (typeof profile.name !== "string" || profile.name.trim() !== profile.name || !profile.name) {
      return null;
    }
    const layout = parseGeometry(profile.layout);
    if (layout === null || names.has(profile.name)) return null;
    names.add(profile.name);
    profiles.push({ name: profile.name, layout });
  }
  const activeProfile = candidate.activeProfile ?? null;
  if (activeProfile !== null && (typeof activeProfile !== "string" || !names.has(activeProfile))) {
    return null;
  }
  return { ...geometry, profiles, activeProfile };
}
