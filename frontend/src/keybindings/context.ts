import { createStore } from "zustand/vanilla";
import type { CommandContext, CommandContextValue } from "../commands/context";

export const BUILTIN_CONTEXT = {
  editorTextFocus: false,
  editorFocus: false,
  activeEditor: false,
  editorReadonly: false,
  editorHasSelection: false,
  editorLangId: "",
  terminalFocus: false,
  panelFocus: false,
  sidebarFocus: false,
  inSearch: false,
  inputFocus: false,
  debugActive: false,
  workspaceOpen: false,
  notificationCenterFocus: false,
  keybindingEditorFocus: false,
  commandPaletteVisible: false,
  vimMode: "normal",
} satisfies Record<string, CommandContextValue>;

export class ContextKeyService {
  readonly store = createStore<Record<string, CommandContextValue>>()(() => ({
    ...BUILTIN_CONTEXT,
  }));

  set(values: CommandContext): void {
    this.store.setState(values);
  }

  focus(target: EventTarget | null): void {
    const element = target instanceof Element ? target : null;
    const editable =
      element instanceof HTMLElement &&
      (element.isContentEditable || element.matches("input, textarea, select"));
    const editor = element?.closest('[data-keybinding-context="editor"]');
    if (editor) this.set({ activeEditor: true });
    this.set({
      editorTextFocus: !!editor && !!editable,
      editorFocus: !!editor,
      inputFocus: !!editable,
      editorHasSelection:
        element instanceof HTMLTextAreaElement && element.selectionStart !== element.selectionEnd,
      editorReadonly: element instanceof HTMLTextAreaElement && element.readOnly,
      editorLangId: editor?.getAttribute("data-language-id") ?? "",
      terminalFocus: !!element?.closest('[data-keybinding-context="terminal"]'),
      panelFocus: !!element?.closest('.workbench-panel, [data-keybinding-context="panel"]'),
      sidebarFocus: !!element?.closest('.workbench-sidebar, [data-keybinding-context="sidebar"]'),
      inSearch: !!element?.closest('[data-keybinding-context="search"], input[type="search"]'),
      notificationCenterFocus: !!element?.closest(".notification-center"),
      keybindingEditorFocus: !!element?.closest(".keybinding-editor"),
      commandPaletteVisible: !!document.querySelector(".command-palette"),
    });
  }
}
