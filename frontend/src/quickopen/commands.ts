import type { QuickOpenQuery } from "../generated/QuickOpenQuery";
import type { QuickOpenResponse } from "../generated/QuickOpenResponse";
import type { InvokeOptions, IpcClient } from "../ipc";

export const QUICK_OPEN_COMMAND = "search.quickOpen";

export function queryQuickOpen(
  client: IpcClient,
  query: QuickOpenQuery,
  options?: InvokeOptions,
): Promise<QuickOpenResponse> {
  return client.invoke<QuickOpenQuery, QuickOpenResponse>(QUICK_OPEN_COMMAND, query, options);
}
