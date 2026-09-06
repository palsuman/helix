import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CommandPalette } from "../commands/CommandPalette";
import { KeybindingEditor } from "./KeybindingEditor";
import { keybindingHarness } from "./testUtils";

describe("Keyboard Shortcuts editor", () => {
  it("searches, filters by command, records a chord and saves it through IPC", async () => {
    const kernel = keybindingHarness();
    await kernel.service.refresh();
    render(<KeybindingEditor service={kernel.service} />);
    fireEvent.change(screen.getByRole("searchbox", { name: "Search shortcuts" }), {
      target: { value: "format" },
    });
    expect(screen.getAllByRole("row")).toHaveLength(2);
    fireEvent.change(screen.getByRole("combobox", { name: "Filter by command" }), {
      target: { value: "editor.action.formatDocument" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Change shortcut for Format Document" }));
    const input = screen.getByRole("textbox", { name: "Shortcut" });
    fireEvent.click(screen.getByRole("button", { name: "Record shortcut" }));
    fireEvent.keyDown(input, { key: "k", metaKey: true });
    fireEvent.keyDown(input, { key: "f", metaKey: true });
    expect(input).toHaveValue("meta+k meta+f");
    fireEvent.click(screen.getByRole("button", { name: "Record shortcut" }));
    fireEvent.click(screen.getByRole("button", { name: "Save binding" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("form", { name: "Edit shortcut for Format Document" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      kernel
        .document()
        .user.some(
          (rule) => rule.key === "meta+k meta+f" && rule.command === "editor.action.formatDocument",
        ),
    ).toBe(true);
    expect(kernel.requests.some((request) => request.command === "keybindings.set")).toBe(true);
  });

  it("names both commands in conflicts and can remove the competing shortcut", async () => {
    const kernel = keybindingHarness("mac", [
      { key: "meta+shift+n", command: "editor.action.formatDocument" },
    ]);
    await kernel.service.refresh();
    render(<KeybindingEditor service={kernel.service} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Conflicts only" }));
    const table = screen.getByRole("table", { name: "Keybindings" });
    expect(table).toHaveTextContent("Format Document");
    expect(table).toHaveTextContent("New Window");
    expect(screen.getAllByRole("row")).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "Change shortcut for Format Document" }));
    const competing = within(screen.getByRole("region", { name: "Conflicting shortcuts" }));
    expect(competing.getByText("New Window", { exact: false })).toBeInTheDocument();
    fireEvent.click(competing.getByRole("button", { name: "Remove competing shortcut" }));
    await waitFor(() => expect(screen.getByText("No matching shortcuts")).toBeInTheDocument());
    expect(
      screen.getByRole("form", { name: "Edit shortcut for Format Document" }),
    ).toBeInTheDocument();
    expect(kernel.service.shortcutFor("workbench.action.newWindow")).toBeNull();
  });

  it("imports a scheme, restores a command, and rejects invalid definitions", async () => {
    const kernel = keybindingHarness();
    await kernel.service.refresh();
    render(<KeybindingEditor service={kernel.service} />);
    fireEvent.change(screen.getByRole("combobox", { name: "Keymap scheme" }), {
      target: { value: "JetBrains" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Import scheme" }));
    await waitFor(() =>
      expect(kernel.service.shortcutFor("editor.action.formatDocument")).toBe("alt+meta+l"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Reset shortcuts for Format Document" }));
    await waitFor(() =>
      expect(kernel.service.shortcutFor("editor.action.formatDocument")).toBe("alt+shift+f"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Change shortcut for Format Document" }));
    fireEvent.change(screen.getByRole("textbox", { name: "When clause" }), {
      target: { value: "editorTextFocus &&" },
    });
    expect(screen.getByRole("button", { name: "Save binding" })).toBeDisabled();
  });

  it("uses reconfigured global shortcuts without the old palette listener", async () => {
    const kernel = keybindingHarness("mac", [
      { key: "meta+shift+p", command: "-workbench.action.showCommands" },
      { key: "meta+y", command: "workbench.action.showCommands" },
    ]);
    const stop = kernel.service.start();
    try {
      render(<CommandPalette registry={kernel.registry} />);
      await act(async () => {
        await kernel.service.refresh();
      });
      fireEvent.keyDown(window, { key: "p", metaKey: true, shiftKey: true });
      expect(screen.queryByRole("dialog", { name: "Command palette" })).not.toBeInTheDocument();
      fireEvent.keyDown(window, { key: "y", metaKey: true });
      expect(await screen.findByRole("dialog", { name: "Command palette" })).toBeInTheDocument();
    } finally {
      stop();
    }
  });
});
