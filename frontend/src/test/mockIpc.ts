import type { AppError } from "../generated/AppError";
import type { IpcRequest } from "../generated/IpcRequest";
import { IpcClient, type InvokeFn } from "../ipc";

export function createMockIpc() {
  const requests: IpcRequest<unknown>[] = [];
  const commands: string[] = [];
  const failures = new Map<string, AppError>();
  const handlers = new Map<string, (request: IpcRequest<unknown>) => unknown>();
  let sequence = 0;
  const invoke: InvokeFn = async <Result>(endpoint: string, args?: Record<string, unknown>) => {
    if (endpoint !== "ipc_dispatch") throw new Error(`Unexpected IPC endpoint: ${endpoint}`);
    const request = (args as { request: IpcRequest<unknown> }).request;
    requests.push(request);
    commands.push(request.command);
    const error = failures.get(request.command);
    if (error) return { correlation_id: request.correlation_id, result: null, error } as Result;
    const handler = handlers.get(request.command);
    if (!handler) throw new Error(`No mock IPC handler for ${request.command}`);
    const response = await handler(request);
    return { correlation_id: request.correlation_id, result: response, error: null } as Result;
  };
  const client = new IpcClient({ invoke, correlationIdFactory: () => `mock-${++sequence}` });

  return {
    client,
    requests,
    commands,
    handle<Request, Response>(
      command: string,
      handler: (payload: Request, request: IpcRequest<Request>) => Response | Promise<Response>,
    ) {
      failures.delete(command);
      handlers.set(command, (request) =>
        handler(request.payload as Request, request as IpcRequest<Request>),
      );
    },
    respond<Response>(command: string, response: Response) {
      failures.delete(command);
      handlers.set(command, () => response);
    },
    fail(command: string, error: AppError) {
      failures.set(command, error);
    },
  };
}
