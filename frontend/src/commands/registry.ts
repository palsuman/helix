import type { CommandDescriptor } from "../generated/CommandDescriptor";
import type { CommandExecuteRequest } from "../generated/CommandExecuteRequest";
import type { CommandExecuteResponse } from "../generated/CommandExecuteResponse";
import type { CommandListRequest } from "../generated/CommandListRequest";
import type { CommandListResponse } from "../generated/CommandListResponse";
import type { CommandRegisterRequest } from "../generated/CommandRegisterRequest";
import type { CommandRegisterResponse } from "../generated/CommandRegisterResponse";
import type { CommandUnregisterRequest } from "../generated/CommandUnregisterRequest";
import type { CommandUnregisterResponse } from "../generated/CommandUnregisterResponse";
import type { IpcClient } from "../ipc";
import { message } from "../localization/message";

export const COMMANDS = {
  list: "command.list",
  register: "command.register",
  unregister: "command.unregister",
  execute: "command.execute",
} as const;

export type CommandHandler = (argumentsValue: unknown) => unknown | Promise<unknown>;

export class CommandRegistry {
  private readonly handlers = new Map<string, CommandHandler>();
  private readonly recentIds: string[] = [];
  private readonly client: IpcClient;
  private readonly windowId: string;

  constructor(client: IpcClient, windowId: string) {
    this.client = client;
    this.windowId = windowId;
  }

  registerHandler(id: string, handler: CommandHandler): () => void {
    this.handlers.set(id, handler);
    return () => {
      if (this.handlers.get(id) === handler) this.handlers.delete(id);
    };
  }

  hasHandler(id: string): boolean {
    return this.handlers.has(id);
  }

  async list(): Promise<CommandDescriptor[]> {
    const response = await this.client.invoke<CommandListRequest, CommandListResponse>(
      COMMANDS.list,
      {},
      { windowId: this.windowId },
    );
    return response.commands;
  }

  async register(command: CommandDescriptor): Promise<boolean> {
    const response = await this.client.invoke<CommandRegisterRequest, CommandRegisterResponse>(
      COMMANDS.register,
      { command },
      { windowId: this.windowId },
    );
    return response.replaced;
  }

  async unregister(id: string): Promise<boolean> {
    const response = await this.client.invoke<CommandUnregisterRequest, CommandUnregisterResponse>(
      COMMANDS.unregister,
      { id },
      { windowId: this.windowId },
    );
    return response.removed;
  }

  async execute(id: string, argumentsValue: unknown = null): Promise<unknown> {
    const response = await this.client.invoke<CommandExecuteRequest, CommandExecuteResponse>(
      COMMANDS.execute,
      { id, arguments: argumentsValue },
      { windowId: this.windowId },
    );
    let result: unknown;
    if (response.target.kind === "renderer") {
      const handler = this.handlers.get(response.id);
      if (handler === undefined)
        throw new Error(message("commandRendererMissing", { command: response.id }));
      result = await handler(response.arguments);
    } else {
      result = await this.client.invoke(response.target.command, response.arguments, {
        windowId: this.windowId,
      });
    }
    this.recordRecent(response.id);
    return result;
  }

  recent(): readonly string[] {
    return this.recentIds;
  }

  private recordRecent(id: string) {
    if (id === "workbench.action.showCommands") return;
    const existing = this.recentIds.indexOf(id);
    if (existing >= 0) this.recentIds.splice(existing, 1);
    this.recentIds.unshift(id);
    this.recentIds.splice(50);
  }
}
