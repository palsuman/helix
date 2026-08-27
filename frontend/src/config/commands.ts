import type { ConfigGetResponse } from "../generated/ConfigGetResponse";
import type { ConfigScope } from "../generated/ConfigScope";
import type { ConfigWriteResponse } from "../generated/ConfigWriteResponse";
import type { InvokeOptions, IpcClient } from "../ipc";
import { ipc } from "../ipc";

/**
 * Typed wrappers for the `config.*` commands (Task 1.6 surface).
 *
 * Kept beside the theme service rather than in `ipc/commands.ts` because the
 * request shapes carry optional scoping fields that only domain callers use.
 */

export const CONFIG_COMMANDS = {
  get: "config.get",
  set: "config.set",
} as const;

export interface ConfigGetOptions extends InvokeOptions {
  language?: string;
}

/** Read one setting's effective value; `null` when nothing sets it. */
export async function getConfigValue<T = unknown>(
  key: string,
  options?: ConfigGetOptions,
): Promise<T | null> {
  return getConfigValueFrom(ipc, key, options);
}

export async function getConfigValueFrom<T = unknown>(
  client: IpcClient,
  key: string,
  options?: ConfigGetOptions,
): Promise<T | null> {
  const response = await client.invoke<Record<string, unknown>, ConfigGetResponse>(
    CONFIG_COMMANDS.get,
    {
      key,
      language: options?.language ?? null,
      workspace_key: null,
      path: null,
    },
    options,
  );
  return (response.setting?.value as T | undefined) ?? null;
}

export async function setConfigValue(
  scope: ConfigScope,
  key: string,
  value: unknown,
  options?: InvokeOptions,
): Promise<ConfigWriteResponse> {
  return setConfigValueOn(ipc, scope, key, value, options);
}

export async function setConfigValueOn(
  client: IpcClient,
  scope: ConfigScope,
  key: string,
  value: unknown,
  options?: InvokeOptions,
): Promise<ConfigWriteResponse> {
  return client.invoke<
    Record<string, unknown>,
    ConfigWriteResponse
  >(CONFIG_COMMANDS.set, { scope, key, value, language: null, workspace_key: null, path: null }, options);
}
