import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type { CommandDescriptor } from "../generated/CommandDescriptor";
import type { IpcRequest } from "../generated/IpcRequest";
import { IpcClient, type InvokeFn } from "../ipc";
import { useNotificationStore } from "../notifications";
import { CommandPalette } from "./CommandPalette";
import { COMMANDS, CommandRegistry } from "./registry";

const commands: CommandDescriptor[] = [
  {
    id: "editor.action.formatDocument",
    title: "Format Document",
    category: "Editor",
    enablement: "editorTextFocus",
    disabled_reason: "Open a text editor to format a document.",
    keybinding: "Shift+Alt+F",
    source: "Helix",
    target: { kind: "renderer" },
  },
  {
    id: "view.toggle",
    title: "Toggle Panel",
    category: "View",
    enablement: null,
    disabled_reason: null,
    keybinding: null,
    source: "Helix",
    target: { kind: "renderer" },
  },
];

function harness() {
  const executed: string[] = [];
  const invoke: InvokeFn = async <T,>(_endpoint: string, args?: Record<string, unknown>) => {
    const request = (args as { request: IpcRequest<unknown> }).request;
    const result =
      request.command === COMMANDS.list
        ? { commands }
        : {
            id: (request.payload as { id: string }).id,
            target: { kind: "renderer" },
            arguments: null,
          };
    return { correlation_id: request.correlation_id, result, error: null } as T;
  };
  const registry = new CommandRegistry(new IpcClient({ invoke }), "main");
  registry.registerHandler("editor.action.formatDocument", () => {
    executed.push("editor.action.formatDocument");
  });
  registry.registerHandler("view.toggle", () => executed.push("view.toggle"));
  return { registry, executed };
}

describe("CommandPalette", () => {
  beforeEach(() => useNotificationStore.getState().reset());

  it("opens through its registered command, fuzzy-filters, groups, and shows shortcuts", async () => {
    const { registry } = harness();
    render(<CommandPalette registry={registry} context={{ editorTextFocus: true }} />);

    await act(async () => {
      await registry.execute("workbench.action.showCommands");
    });
    const input = await screen.findByRole("combobox", { name: "Search commands" });
    fireEvent.change(input, { target: { value: "form" } });

    const editorGroup = screen.getByRole("group", { name: "Editor" });
    expect(within(editorGroup).getByRole("option")).toHaveTextContent("Format Document");
    expect(within(editorGroup).getByText("Shift+Alt+F")).toBeInTheDocument();
    expect(screen.queryByText("Toggle Panel")).not.toBeInTheDocument();
  });

  it("shows disabled context reasons and refuses execution", async () => {
    const { registry, executed } = harness();
    render(<CommandPalette registry={registry} context={{ editorTextFocus: false }} />);
    await act(async () => {
      await registry.execute("workbench.action.showCommands");
    });
    const input = await screen.findByRole("combobox", { name: "Search commands" });
    fireEvent.change(input, { target: { value: "form" } });

    const option = screen.getByRole("option", { name: /Format Document/ });
    expect(option).toHaveAttribute("aria-disabled", "true");
    expect(option).toHaveTextContent("Open a text editor to format a document.");
    fireEvent.click(option);
    expect(executed).toEqual([]);
  });

  it("executes with Enter, closes, and places the command first on reopen", async () => {
    const { registry, executed } = harness();
    render(<CommandPalette registry={registry} context={{ editorTextFocus: true }} />);
    await act(async () => {
      await registry.execute("workbench.action.showCommands");
    });
    const input = await screen.findByRole("combobox", { name: "Search commands" });
    fireEvent.change(input, { target: { value: "form" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Command palette" })).not.toBeInTheDocument(),
    );
    expect(executed).toEqual(["editor.action.formatDocument"]);

    await act(async () => {
      await registry.execute("workbench.action.showCommands");
    });
    await screen.findByRole("combobox", { name: "Search commands" });
    const options = await screen.findAllByRole("option");
    expect(options[0]).toHaveTextContent("Format Document");
  });

  it("restores focus on Escape", async () => {
    const { registry } = harness();
    render(
      <>
        <button type="button">Before</button>
        <CommandPalette registry={registry} />
      </>,
    );
    const before = screen.getByRole("button", { name: "Before" });
    before.focus();
    await act(async () => {
      await registry.execute("workbench.action.showCommands");
    });
    const input = await screen.findByRole("combobox", { name: "Search commands" });
    fireEvent.keyDown(input, { key: "Escape" });
    await act(async () => new Promise(requestAnimationFrame));
    expect(before).toHaveFocus();
  });
});
