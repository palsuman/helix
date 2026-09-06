import { useState } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NotificationCenter, NotificationCenterButton, NotificationToasts } from "./Notifications";
import { notify, useNotificationStore } from "./notificationStore";

function NotificationSurface() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <NotificationToasts centerVisible={open} />
      <NotificationCenterButton open={open} onToggle={() => setOpen(!open)} />
      {open && <NotificationCenter onClose={() => setOpen(false)} />}
    </>
  );
}

function renderNotifications() {
  return render(<NotificationSurface />);
}

function publish(input: Parameters<typeof notify>[0]) {
  act(() => {
    notify(input);
  });
}

describe("notifications", () => {
  beforeEach(() => useNotificationStore.getState().reset());

  it("announces and renders source-attributed notifications", () => {
    renderNotifications();
    publish({ kind: "warning", message: "Indexing is slow", source: "Search" });

    expect(screen.getByRole("status")).toHaveTextContent("Search: Indexing is slow");
    const toasts = screen.getByRole("region", { name: "Notifications" });
    expect(within(toasts).getByText("Indexing is slow")).toBeInTheDocument();
    expect(within(toasts).getByText("Search")).toBeInTheDocument();
  });

  it("dispatches actions and progress cancellation", async () => {
    const retry = vi.fn();
    const cancel = vi.fn();
    renderNotifications();
    publish({
      kind: "progress",
      message: "Building workspace",
      source: "Tasks",
      progress: 45,
      actions: [{ label: "Open output", run: retry }],
      cancel,
    });

    expect(
      screen.getByRole("progressbar", { name: "Building workspace progress" }),
    ).toHaveAttribute("aria-valuenow", "45");
    fireEvent.click(screen.getByRole("button", { name: "Open output" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(retry).toHaveBeenCalledOnce());
    expect(cancel).toHaveBeenCalledOnce();
  });

  it("suppresses toasts in do-not-disturb while retaining center history", () => {
    renderNotifications();
    fireEvent.click(screen.getByRole("button", { name: "Notification center, 0 notifications" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Do not disturb" }));
    publish({ kind: "error", message: "Connection failed", source: "Kernel" });

    expect(screen.queryByRole("region", { name: "Notifications" })?.children).toHaveLength(0);
    expect(screen.getByRole("region", { name: "Notification center" })).toHaveTextContent(
      "Connection failed",
    );
    expect(
      screen.getByRole("button", { name: "Notification center, 1 notification" }),
    ).toBeInTheDocument();
  });

  it("keeps dismissed notifications in the center until history is cleared", () => {
    renderNotifications();
    publish({ kind: "error", message: "Save failed", source: "File system" });
    fireEvent.click(screen.getByRole("button", { name: "Dismiss notification" }));

    expect(
      within(screen.getByRole("region", { name: "Notifications" })).queryByText("Save failed"),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Notification center, 1 notification" }));
    expect(screen.getByRole("region", { name: "Notification center" })).toHaveTextContent(
      "Save failed",
    );
    fireEvent.click(screen.getByRole("button", { name: "Clear all notifications" }));
    expect(screen.getByText("No notifications")).toBeInTheDocument();
  });

  it("renders indeterminate progress without a numeric value", () => {
    renderNotifications();
    publish({ kind: "progress", message: "Scanning", source: "Workspace", progress: null });

    expect(screen.getByRole("progressbar", { name: "Scanning progress" })).not.toHaveAttribute(
      "aria-valuenow",
    );
  });

  it("keeps history and DND when hidden with Escape or the header button", () => {
    renderNotifications();
    publish({ kind: "error", message: "Save failed", source: "File system" });
    const toggle = screen.getByRole("button", { name: "Notification center, 1 notification" });
    fireEvent.click(toggle);
    const dnd = screen.getByRole("checkbox", { name: "Do not disturb" });
    fireEvent.click(dnd);
    dnd.focus();
    fireEvent.keyDown(dnd, { key: "Escape" });

    expect(screen.queryByRole("region", { name: "Notification center" })).not.toBeInTheDocument();
    expect(toggle).toHaveFocus();
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(toggle).toHaveAttribute("title", "Notifications (do not disturb)");
    fireEvent.click(toggle);
    expect(screen.getByRole("log", { name: "Notification history" })).toHaveTextContent(
      "Save failed",
    );
    expect(screen.getByRole("checkbox", { name: "Do not disturb" })).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "Hide notification center" }));
    expect(toggle).toHaveFocus();
    expect(useNotificationStore.getState().entries).toHaveLength(1);
  });

  it("renders newest-first history and runs actions without covering the open panel with toasts", async () => {
    const openOutput = vi.fn();
    const cancel = vi.fn();
    renderNotifications();
    publish({ kind: "info", message: "Workspace opened", source: "Workspace" });
    publish({
      kind: "progress",
      message: "Building workspace",
      source: "Tasks",
      progress: 45,
      actions: [{ label: "Open output", run: openOutput }],
      cancel,
    });
    fireEvent.click(screen.getByRole("button", { name: "Notification center, 2 notifications" }));

    const history = within(screen.getByRole("log", { name: "Notification history" }));
    expect(history.getAllByRole("article")[0]).toHaveTextContent("Building workspace");
    expect(history.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "45");
    expect(screen.getByRole("region", { name: "Notifications" }).children).toHaveLength(0);
    expect(screen.queryByRole("dialog", { name: "Notification center" })).not.toBeInTheDocument();
    fireEvent.click(history.getByRole("button", { name: "Open output" }));
    fireEvent.click(history.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(openOutput).toHaveBeenCalledOnce());
    expect(cancel).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Clear all notifications" }));
    expect(screen.getByRole("button", { name: "Clear all notifications" })).toBeDisabled();
    expect(screen.getByText("No notifications")).toBeInTheDocument();
  });
});
