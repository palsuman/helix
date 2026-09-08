import { describe, expect, it } from "vitest";
import { createMockIpc } from "../test/mockIpc";
import {
  BufferLifecycle,
  applySaveResult,
  bufferFromFile,
  classifyDropPath,
  createUntitledBuffer,
  hasConflictMarkers,
  markExternallyDeleted,
  normalizeText,
  shouldAutoSave,
  updateBufferText,
} from "./lifecycle";

function wal() {
  const buffers = new Map<string, ReturnType<typeof createUntitledBuffer>>();
  return {
    load: async () => [...buffers.values()],
    put: async (buffer: ReturnType<typeof createUntitledBuffer>) => void buffers.set(buffer.id, buffer),
    remove: async (id: string) => void buffers.delete(id),
  };
}

describe("editor file lifecycle", () => {
  it("restores untitled buffers through the WAL and normalizes saves", async () => {
    const persistence = wal();
    const buffer = createUntitledBuffer("typescript", "const x = 1  ");
    await persistence.put(buffer);
    const kernel = createMockIpc();
    kernel.respond("fs.write", {
      outcome: { path: "/tmp/x.ts", bytes_written: 12n, hash: "new", encoding: "utf8", eol: "lf", lossy: false },
    });
    const lifecycle = new BufferLifecycle(kernel.client, persistence, { saveAs: async () => "/tmp/x.ts" }, { trimTrailingWhitespace: true, insertFinalNewline: true });
    expect(await lifecycle.restore()).toHaveLength(1);
    const restored = lifecycle.get(buffer.id)!;
    lifecycle.update(updateBufferText(restored, "const x = 1  "));
    const outcome = await lifecycle.save(buffer.id);
    expect(outcome?.buffer.path).toBe("/tmp/x.ts");
    expect(kernel.requests[0]?.payload).toMatchObject({ text: "const x = 1\n" });
  });

  it("saveAll isolates failures and rejects Save As path collisions", async () => {
    const kernel = createMockIpc();
    kernel.handle("fs.write", (payload: { path: string }) => {
      if (payload.path === "/bad.ts") throw new Error("disk full");
      return { outcome: { path: payload.path, bytes_written: 1n, hash: "h", encoding: "utf8", eol: "lf", lossy: false } };
    });
    const lifecycle = new BufferLifecycle(kernel.client, wal(), { saveAs: async () => "/good.ts" });
    const good = lifecycle.add(createUntitledBuffer("typescript", "ok"));
    lifecycle.add({ ...createUntitledBuffer("typescript", "bad"), id: "bad", path: "/bad.ts" });
    const result = await lifecycle.saveAll();
    expect(result.saved).toHaveLength(1);
    expect(result.errors).toHaveLength(1);
    lifecycle.add({ ...good, id: "other", path: "/good.ts", dirty: true });
    await expect(lifecycle.save("other")).rejects.toThrow("EDITOR_PATH_COLLISION");
  });

  it("covers conflict, autosave, deletion, encoding, EOL, and drops", () => {
    const file = bufferFromFile({ path: "/a.ts", text: "x", encoding: "utf8", encoding_from_bom: false, eol: { style: "crlf", lf_count: 0, crlf_count: 1 }, binary: false, hash: "h", size: 1n, readonly: false, modified_ms: null }, "typescript");
    const dirty = updateBufferText(file, "<<<<<<< ours\nvalue\n=======\nother\n>>>>>>> theirs");
    expect(hasConflictMarkers(dirty.text)).toBe(true);
    expect(shouldAutoSave({ ...dirty, autoSave: "afterDelay" }, "afterDelay")).toBe(false);
    expect(markExternallyDeleted(file).dirtyWithNoFile).toBe(true);
    expect(applySaveResult(file, { path: "/b.ts", bytes_written: 1n, hash: "n", encoding: "utf8", eol: "lf", lossy: false }).path).toBe("/b.ts");
    expect(normalizeText("a  \r\n", { trimTrailingWhitespace: true })).toBe("a\n");
    expect(classifyDropPath("/tmp/project", true)).toBe("workspace-root");
    expect(classifyDropPath("/tmp/project/a.ts", false)).toBe("editor");
  });
});