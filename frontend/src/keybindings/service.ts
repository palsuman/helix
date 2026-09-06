import { createStore } from "zustand/vanilla";
import { evaluateEnablement, type CommandContext } from "../commands/context";
import type { CommandRegistry } from "../commands/registry";
import type { CommandDescriptor } from "../generated/CommandDescriptor";
import type { KeybindingsSnapshot } from "../generated/KeybindingsSnapshot";
import type { KeybindingsSetRequest } from "../generated/KeybindingsSetRequest";
import type { KeybindingContribution } from "../generated/KeybindingContribution";
import type { IpcClient } from "../ipc";
import { notify } from "../notifications";
import { ContextKeyService } from "./context";
import {
  KeybindingResolver,
  resolveBindings,
  type KeybindingPlatform,
  type KeybindingRule,
  type ResolvedBinding,
} from "./resolver";
import { defaultBindings, detectPlatform, importScheme, type KeybindingScheme } from "./schemes";

interface KeybindingState {
  document: KeybindingsSnapshot | null;
  bindings: ResolvedBinding[];
  commands: CommandDescriptor[];
  warnings: string[];
  error: string | null;
  pending: string[];
  editorOpen: boolean;
}

export class KeybindingService {
  readonly context = new ContextKeyService();
  readonly store = createStore<KeybindingState>()(() => ({
    document: null,
    bindings: [],
    commands: [],
    warnings: [],
    error: null,
    pending: [],
    editorOpen: false,
  }));
  readonly platform: KeybindingPlatform;
  private readonly ipc: IpcClient;
  private readonly registry: CommandRegistry;
  private readonly windowId: string;
  private readonly resolver: KeybindingResolver;
  private refreshSequence = 0;

  constructor(
    ipc: IpcClient,
    registry: CommandRegistry,
    windowId: string,
    platform = detectPlatform(),
  ) {
    this.ipc = ipc;
    this.registry = registry;
    this.windowId = windowId;
    this.platform = platform;
    this.resolver = new KeybindingResolver((pending) => this.store.setState({ pending }));
    this.rebuild(null);
  }

  private rebuild(document: KeybindingsSnapshot | null): void {
    const result = resolveBindings(
      [
        { source: "default", owner: "Helix", rules: defaultBindings(this.platform) },
        ...(document?.plugins ?? []).map((plugin) => ({ source: "plugin" as const, ...plugin })),
        { source: "user", owner: "User", rules: document?.user ?? [] },
      ],
      this.platform,
    );
    const warnings = [...(document?.warnings ?? []), ...result.warnings];
    if (warnings.length && warnings.join("\n") !== this.store.getState().warnings.join("\n")) {
      notify({ kind: "warning", source: "Keybindings", message: warnings.join(" ") });
    }
    this.resolver.reset();
    this.store.setState({ document, bindings: result.bindings, warnings, error: null });
  }

  async refresh(): Promise<void> {
    const sequence = ++this.refreshSequence;
    try {
      const [document, commands] = await Promise.all([
        this.ipc.invoke<Record<string, never>, KeybindingsSnapshot>(
          "keybindings.get",
          {},
          { windowId: this.windowId },
        ),
        this.registry.list(),
      ]);
      if (sequence !== this.refreshSequence) return;
      this.store.setState({ commands, error: null });
      if (JSON.stringify(document) !== JSON.stringify(this.store.getState().document))
        this.rebuild(document);
    } catch (error) {
      if (sequence === this.refreshSequence) this.store.setState({ error: String(error) });
    }
  }

  async save(rules: KeybindingRule[]): Promise<void> {
    const current = this.store.getState().document;
    if (!current?.writable)
      throw new Error("Keybindings are unavailable or read-only. Reload after repairing the file.");
    ++this.refreshSequence;
    const document = await this.ipc.invoke<KeybindingsSetRequest, KeybindingsSnapshot>(
      "keybindings.set",
      { rules, revision: current.revision },
      { windowId: this.windowId },
    );
    ++this.refreshSequence;
    this.rebuild(document);
  }

  async contribute(owner: string, rules: KeybindingRule[]): Promise<void> {
    const document = await this.ipc.invoke<KeybindingContribution, KeybindingsSnapshot>(
      "keybindings.contribute",
      { owner, rules },
      { windowId: this.windowId },
    );
    this.rebuild(document);
  }

  async removeContribution(owner: string): Promise<void> {
    const document = await this.ipc.invoke<{ owner: string }, KeybindingsSnapshot>(
      "keybindings.removeContribution",
      { owner },
      { windowId: this.windowId },
    );
    this.rebuild(document);
  }

  async import(scheme: KeybindingScheme): Promise<void> {
    const state = this.store.getState();
    await this.save(
      importScheme(state.document?.user ?? [], state.bindings, scheme, this.platform),
    );
  }

  async rebind(command: string, key: string, when?: string, args?: unknown): Promise<void> {
    const state = this.store.getState();
    const removals = state.bindings
      .filter((binding) => binding.command === command && binding.source !== "user")
      .map((binding) => ({
        key: binding.key,
        command: `-${command}`,
        ...(binding.when ? { when: binding.when } : {}),
      }));
    await this.save([
      ...(state.document?.user ?? []).filter((rule) => rule.command !== command),
      ...removals,
      {
        key,
        command,
        ...(when?.trim() ? { when: when.trim() } : {}),
        ...(args !== undefined ? { args } : {}),
      },
    ]);
  }

  async remove(binding: ResolvedBinding): Promise<void> {
    const rules = [...(this.store.getState().document?.user ?? [])];
    if (binding.source === "user") rules.splice(Number(binding.id.split(":").at(-1)), 1);
    else
      rules.push({
        key: binding.key,
        command: `-${binding.command}`,
        ...(binding.when ? { when: binding.when } : {}),
      });
    await this.save(rules);
  }

  async resetCommand(command: string): Promise<void> {
    await this.save(
      (this.store.getState().document?.user ?? []).filter(
        (rule) => rule.command !== command && rule.command !== `-${command}`,
      ),
    );
  }

  canExecute(command: string, context: CommandContext): boolean {
    const descriptor = this.store.getState().commands.find((candidate) => candidate.id === command);
    return (
      descriptor !== undefined &&
      evaluateEnablement(descriptor.enablement, context) &&
      (descriptor.target.kind !== "renderer" || this.registry.hasHandler(command))
    );
  }

  shortcutFor(command: string, context?: CommandContext): string | null {
    return (
      [...this.store.getState().bindings]
        .reverse()
        .find(
          (binding) =>
            binding.command === command && (!context || evaluateEnablement(binding.when, context)),
        )?.key ?? null
    );
  }

  handle(event: KeyboardEvent): boolean {
    if (event.target instanceof Element && event.target.closest('[data-shortcut-recorder="true"]'))
      return false;
    this.context.focus(event.target);
    const context = this.context.store.getState();
    const result = this.resolver.handle(event, this.store.getState().bindings, context, (command) =>
      this.canExecute(command, context),
    );
    if (!result.consumed) return false;
    event.preventDefault();
    if (result.binding) {
      void this.registry
        .execute(result.binding.command, result.binding.args ?? null)
        .catch((error: unknown) => {
          this.store.setState({ error: String(error) });
          notify({ kind: "error", source: "Keybindings", message: String(error) });
        });
    }
    return true;
  }

  start(): () => void {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const refresh = async () => {
      await this.refresh();
      if (!stopped) timer = setTimeout(() => void refresh(), 1_000);
    };
    const keydown = (event: KeyboardEvent) => {
      this.handle(event);
    };
    const focus = (event: FocusEvent) => this.context.focus(event.target);
    const cancel = () => this.resolver.reset();
    const unregister = this.registry.registerHandler("workbench.action.openGlobalKeybindings", () =>
      this.store.setState({ editorOpen: true }),
    );
    window.addEventListener("keydown", keydown);
    window.addEventListener("focusin", focus);
    window.addEventListener("blur", cancel);
    document.addEventListener("visibilitychange", cancel);
    void refresh();
    return () => {
      stopped = true;
      ++this.refreshSequence;
      clearTimeout(timer);
      cancel();
      unregister();
      window.removeEventListener("keydown", keydown);
      window.removeEventListener("focusin", focus);
      window.removeEventListener("blur", cancel);
      document.removeEventListener("visibilitychange", cancel);
    };
  }
}
