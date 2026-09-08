import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FsReadResponse } from "../generated/FsReadResponse";
import type { FsWriteResponse } from "../generated/FsWriteResponse";
import { createMockIpc } from "../test/mockIpc";
import { EditorSurface, type MonacoLoader } from "./EditorSurface";

const monacoMock = vi.hoisted(() => {
  const model = {
    getValue: vi.fn(() => "edited"),
    onDidChangeContent: vi.fn(() => ({ dispose: vi.fn() })),
    setValue: vi.fn(),
    dispose: vi.fn(),
  };
  const editor = {
    addCommand: vi.fn(),
    dispose: vi.fn(),
  };
  return {
    model,
    editor,
    editorApi: {
      createModel: vi.fn(() => model),
      create: vi.fn(() => editor),
    },
    Uri: { parse: vi.fn((value: string) => value) },
    KeyMod: { CtrlCmd: 1 },
    KeyCode: { KeyS: 2, KeyF: 3, KeyH: 4 },
  };
});

const monacoLoader = async () =>
  ({
    editor: monacoMock.editorApi,
    Uri: monacoMock.Uri,
    KeyMod: monacoMock.KeyMod,
    KeyCode: monacoMock.KeyCode,
  }) as unknown as Awaited<ReturnType<MonacoLoader>>;

function content(overrides: Partial<FsReadResponse["content"]> = {}): FsReadResponse {
  return {
    content: {
      path: "/workspace/main.ts",
      text: "const answer = 42;",
      encoding: "utf8",
      encoding_from_bom: false,
      eol: { style: "lf", lf_count: 1, crlf_count: 0 },
      binary: false,
      hash: "hash-before",
      size: 19n,
      readonly: false,
      modified_ms: null,
      ...overrides,
    },
  };
}

describe("EditorSurface", () => {
  beforeEach(() => {
    monacoMock.editorApi.createModel.mockClear();
    monacoMock.editorApi.create.mockClear();
    monacoMock.editor.addCommand.mockClear();
    monacoMock.editor.dispose.mockClear();
    monacoMock.model.getValue.mockReturnValue("edited");
    monacoMock.model.onDidChangeContent.mockClear();
    monacoMock.model.dispose.mockClear();
  });

  afterEach(() => vi.restoreAllMocks());

  it("opens a file, saves edits with its expected hash, and disposes the model", async () => {
    const kernel = createMockIpc();
    kernel.respond("fs.read", content());
    const write: FsWriteResponse = {
      outcome: {
        path: "/workspace/main.ts",
        bytes_written: 6n,
        hash: "hash-after",
        encoding: "utf8",
        eol: "lf",
        lossy: false,
      },
    };
    kernel.respond("fs.write", write);
    const onDirtyChange = vi.fn();
    const onSaved = vi.fn();

    const view = render(
      <EditorSurface
        client={kernel.client}
        monacoLoader={monacoLoader}
        path="/workspace/main.ts"
        onDirtyChange={onDirtyChange}
        onSaved={onSaved}
      />,
    );
    await waitFor(() => expect(monacoMock.editorApi.create).toHaveBeenCalled());

    const listenerArgs = monacoMock.model.onDidChangeContent.mock.calls[0] as unknown as
      | [() => void]
      | undefined;
    const contentListener = listenerArgs?.[0];
    expect(contentListener).toBeDefined();
    contentListener?.();
    await monacoMock.editor.addCommand.mock.calls[0][1]();

    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(false));
    expect(onSaved).toHaveBeenCalledWith(expect.objectContaining({ hash: "hash-after" }));
    expect(kernel.requests).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          command: "fs.write",
          payload: expect.objectContaining({ expected_hash: "hash-before", text: "edited" }),
        }),
      ]),
    );

    view.unmount();
    expect(monacoMock.editor.dispose).toHaveBeenCalled();
    expect(monacoMock.model.dispose).toHaveBeenCalled();
  });

  it("uses large-file options for files at or above 5 MB", async () => {
    const kernel = createMockIpc();
    kernel.respond("fs.read", content({ size: 5n * 1024n * 1024n }));

    render(<EditorSurface client={kernel.client} monacoLoader={monacoLoader} path="/workspace/large.ts" />);
    await waitFor(() => expect(monacoMock.editorApi.create).toHaveBeenCalled());

    expect(monacoMock.editorApi.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({
        folding: false,
        largeFileOptimizations: true,
        minimap: { enabled: false },
        renderWhitespace: "none",
        wordWrap: "off",
      }),
    );
    expect(screen.getByText("Large file mode", { exact: false })).toBeInTheDocument();
  });

  it("refuses binary files without creating an editable model", async () => {
    const kernel = createMockIpc();
    kernel.respond("fs.read", content({ binary: true, text: null }));

    render(<EditorSurface client={kernel.client} monacoLoader={monacoLoader} path="/workspace/image.png" />);
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("Binary files cannot be edited"),
    );
    expect(monacoMock.editorApi.createModel).not.toHaveBeenCalled();
    expect(monacoMock.editorApi.create).not.toHaveBeenCalled();
  });
});