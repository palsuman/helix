import type { Encoding } from "../generated/Encoding";
import type { EolInfo } from "../generated/EolInfo";
import type { FileContent } from "../generated/FileContent";
import type { FsReadResponse } from "../generated/FsReadResponse";
import type { FsWriteResponse } from "../generated/FsWriteResponse";
import type { IpcClient } from "../ipc";

export const FILE_READ_COMMAND = "fs.read";
export const FILE_WRITE_COMMAND = "fs.write";

export interface EditorFile {
  path: string;
  text: string | null;
  encoding: Encoding;
  encoding_from_bom: boolean;
  eol: EolInfo;
  binary: boolean;
  hash: string;
  size: bigint;
  readonly: boolean;
  modified_ms: bigint | null;
}

export interface EditorWriteResult {
  path: string;
  bytes_written: bigint;
  hash: string;
  encoding: Encoding;
  eol: "lf" | "crlf" | "mixed" | "none";
  lossy: boolean;
}

function toEditorFile(content: FileContent): EditorFile {
  return content;
}

export async function readEditorFile(client: IpcClient, path: string): Promise<EditorFile> {
  const response = await client.invoke<{ path: string }, FsReadResponse>(FILE_READ_COMMAND, { path });
  return toEditorFile(response.content);
}

export async function writeEditorFile(
  client: IpcClient,
  file: Pick<EditorFile, "path" | "encoding" | "eol" | "hash">,
  text: string,
): Promise<EditorWriteResult> {
  const response = await client.invoke<
    {
      path: string;
      text: string;
      encoding: Encoding;
      eol: EditorFile["eol"]["style"] | null;
      expected_hash: string | null;
    },
    FsWriteResponse
  >(FILE_WRITE_COMMAND, {
    path: file.path,
    text,
    encoding: file.encoding,
    eol: file.eol.style === "none" ? null : file.eol.style,
    expected_hash: file.hash,
  });
  return response.outcome;
}