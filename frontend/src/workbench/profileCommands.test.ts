import { beforeEach, describe, expect, it } from "vitest";
import { useLayoutStore } from "./layoutStore";
import { executeLayoutProfileCommand } from "./profileCommands";

describe("layout profile command handlers", () => {
  beforeEach(() => useLayoutStore.getState().reset());

  it("exposes save, switch, rename, list, and delete through command handlers", () => {
    executeLayoutProfileCommand("workbench.layoutProfile.save", { name: "Debug" });
    executeLayoutProfileCommand("workbench.layoutProfile.rename", {
      name: "Debug",
      nextName: "Writing",
    });
    expect(executeLayoutProfileCommand("workbench.layoutProfile.list")).toEqual([
      { name: "Writing", active: true },
    ]);
    expect(
      executeLayoutProfileCommand("workbench.layoutProfile.switch", { name: "Writing" }),
    ).toEqual([]);
    executeLayoutProfileCommand("workbench.layoutProfile.delete", { name: "Writing" });
    expect(executeLayoutProfileCommand("workbench.layoutProfile.list")).toEqual([]);
  });

  it("fails command argument errors without changing profile state", () => {
    expect(() => executeLayoutProfileCommand("workbench.layoutProfile.save")).toThrow(
      "requires 'name'",
    );
    expect(useLayoutStore.getState().profiles).toEqual([]);
  });
});
