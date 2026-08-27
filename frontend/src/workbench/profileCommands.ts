import { listLayoutProfiles, useLayoutStore, type ProfileViewAvailability } from "./layoutStore";

export const LAYOUT_PROFILE_COMMANDS = [
  { id: "workbench.layoutProfile.save", title: "Layout Profile: Save" },
  { id: "workbench.layoutProfile.switch", title: "Layout Profile: Switch" },
  { id: "workbench.layoutProfile.rename", title: "Layout Profile: Rename" },
  { id: "workbench.layoutProfile.delete", title: "Layout Profile: Delete" },
  { id: "workbench.layoutProfile.list", title: "Layout Profile: List" },
  { id: "workbench.action.toggleZenMode", title: "View: Toggle Zen Mode" },
] as const;

export type LayoutProfileCommandId = (typeof LAYOUT_PROFILE_COMMANDS)[number]["id"];

export interface LayoutProfileCommandArguments {
  name?: string;
  nextName?: string;
}

const required = (value: string | undefined, field: string) => {
  if (value === undefined) throw new Error(`Layout profile command requires '${field}'.`);
  return value;
};

/** Command handlers consumed by Task 2.8's registry and palette. */
export function executeLayoutProfileCommand(
  id: LayoutProfileCommandId,
  args: LayoutProfileCommandArguments = {},
  available?: ProfileViewAvailability,
): unknown {
  const store = useLayoutStore.getState();
  switch (id) {
    case "workbench.layoutProfile.save":
      store.saveProfile(required(args.name, "name"));
      return undefined;
    case "workbench.layoutProfile.switch":
      return store.switchProfile(required(args.name, "name"), available);
    case "workbench.layoutProfile.rename":
      store.renameProfile(required(args.name, "name"), required(args.nextName, "nextName"));
      return undefined;
    case "workbench.layoutProfile.delete":
      store.deleteProfile(required(args.name, "name"));
      return undefined;
    case "workbench.layoutProfile.list":
      return listLayoutProfiles(store);
    case "workbench.action.toggleZenMode":
      store.toggleZenMode();
      return undefined;
  }
}
