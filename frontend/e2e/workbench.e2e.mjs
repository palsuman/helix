import { browser, $, expect } from "@wdio/globals";
import { closeCleanly, invokeKernel, mockKernelCommands, waitForWorkbench } from "./support.mjs";

describe("Native workbench", () => {
  it("renders, interacts, mocks IPC, restores real IPC, and closes cleanly", async () => {
    await waitForWorkbench();
    for (const label of [
      "Title bar",
      "Left activity rail",
      "Right activity rail",
      "Editor area",
      "Status bar",
    ]) {
      await expect($(`[aria-label="${label}"]`)).toBeDisplayed();
    }
    const toggle = $('[aria-label="Right activity rail"] button');
    await toggle.click();
    await expect($('[aria-label="Right panel"]')).not.toExist();
    await toggle.click();
    await expect($('[aria-label="Right panel"]')).toBeDisplayed();

    const original = await invokeKernel("ipc.ping", { message: "real-kernel" });
    expect(original.echo).toBe("real-kernel");
    const restore = await mockKernelCommands({
      "ipc.ping": { echo: "mocked-kernel", kernel_version: "e2e" },
    });
    try {
      const mocked = await invokeKernel("ipc.ping", { message: "ignored" });
      expect(mocked).toEqual({ echo: "mocked-kernel", kernel_version: "e2e" });
      const commands = await invokeKernel("command.list");
      expect(commands.commands.length).toBeGreaterThan(0);
    } finally {
      await restore();
    }
    expect((await invokeKernel("ipc.ping", { message: "restored" })).echo).toBe("restored");
    if (process.env.HELIX_E2E_SCREENSHOT === "1") {
      await browser.saveScreenshot(`${process.env.HELIX_E2E_HOME}/workbench.png`);
    }
    await closeCleanly();
  });
});
