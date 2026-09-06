import { Icon, type IconId } from "../icons";
import {
  type NotificationEntry,
  type NotificationKind,
  useNotificationStore,
} from "./notificationStore";
import "./notifications.css";

const KIND_ICONS = {
  info: "info",
  warning: "warning",
  error: "error",
  progress: "spinner",
} as const satisfies Record<NotificationKind, IconId>;

function runAction(action: () => void | Promise<void>) {
  void Promise.resolve()
    .then(action)
    .catch((error: unknown) => {
      console.error("[notifications] Action failed", error);
    });
}

function Progress({ entry }: { entry: NotificationEntry }) {
  if (entry.kind !== "progress") return null;
  if (entry.progress === null || entry.progress === undefined) {
    return (
      <div
        className="notification-progress notification-progress--indeterminate"
        role="progressbar"
        aria-label={`${entry.message} progress`}
      />
    );
  }
  return (
    <div
      className="notification-progress"
      role="progressbar"
      aria-label={`${entry.message} progress`}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={entry.progress}
    >
      <span style={{ width: `${entry.progress}%` }} />
    </div>
  );
}

function NotificationContent({
  entry,
  compact = false,
}: {
  entry: NotificationEntry;
  compact?: boolean;
}) {
  const dismissToast = useNotificationStore((state) => state.dismissToast);
  return (
    <article
      className={`notification notification--${entry.kind}${compact ? " notification--compact" : ""}`}
      data-notification-id={entry.id}
    >
      <Icon
        id={KIND_ICONS[entry.kind]}
        spin={entry.kind === "progress" && entry.progress == null}
        className="notification-kind-icon"
      />
      <div className="notification-body">
        <div className="notification-heading">
          <strong>{entry.message}</strong>
          <div className="notification-metadata">
            <span>{entry.source}</span>
            {compact && (
              <time
                dateTime={new Date(entry.createdAt).toISOString()}
                title={new Date(entry.createdAt).toLocaleString()}
              >
                {new Date(entry.createdAt).toLocaleTimeString([], {
                  hour: "2-digit",
                  minute: "2-digit",
                })}
              </time>
            )}
          </div>
        </div>
        <Progress entry={entry} />
        {(entry.actions.length > 0 || entry.cancel !== undefined) && (
          <div className="notification-actions">
            {entry.actions.map((action) => (
              <button type="button" key={action.label} onClick={() => runAction(action.run)}>
                {action.label}
              </button>
            ))}
            {entry.cancel !== undefined && (
              <button type="button" onClick={() => runAction(entry.cancel!)}>
                Cancel
              </button>
            )}
          </div>
        )}
      </div>
      {!compact && (
        <button
          type="button"
          className="notification-dismiss"
          aria-label="Dismiss notification"
          title="Dismiss notification"
          onClick={() => dismissToast(entry.id)}
        >
          <Icon id="close" size="sm" label="Dismiss notification" />
        </button>
      )}
    </article>
  );
}

export function NotificationToasts({ centerVisible = false }: { centerVisible?: boolean }) {
  const entries = useNotificationStore((state) => state.entries);
  const visibleEntries = centerVisible ? [] : entries.filter((entry) => entry.toastVisible);
  const latest = entries.at(-1);

  return (
    <>
      <div className="notification-live-region" role="status" aria-live="polite" aria-atomic="true">
        {latest === undefined ? "" : `${latest.source}: ${latest.message}`}
      </div>
      <section className="notification-toasts" aria-label="Notifications">
        {visibleEntries.map((entry) => (
          <NotificationContent key={entry.id} entry={entry} />
        ))}
      </section>
    </>
  );
}

export function NotificationCenterButton({
  open,
  onToggle,
}: {
  open: boolean;
  onToggle: () => void;
}) {
  const entries = useNotificationStore((state) => state.entries);
  const doNotDisturb = useNotificationStore((state) => state.doNotDisturb);

  return (
    <button
      id="notification-center-toggle"
      type="button"
      className={`notification-center-button${open ? " is-active" : ""}`}
      aria-label={`Notification center, ${entries.length} notifications`}
      title={doNotDisturb ? "Notifications (do not disturb)" : "Notifications"}
      aria-expanded={open}
      aria-pressed={open}
      aria-controls={open ? "notification-center" : undefined}
      onClick={onToggle}
    >
      <Icon id="bell" size="lg" label="Notifications" />
      {entries.length > 0 && (
        <span className="notification-center-badge" aria-hidden="true">
          {entries.length > 99 ? "99+" : entries.length}
        </span>
      )}
      {doNotDisturb && <span className="notification-center-muted" aria-hidden="true" />}
    </button>
  );
}

export function NotificationCenter({ onClose }: { onClose: () => void }) {
  const entries = useNotificationStore((state) => state.entries);
  const doNotDisturb = useNotificationStore((state) => state.doNotDisturb);
  const setDoNotDisturb = useNotificationStore((state) => state.setDoNotDisturb);
  const clear = useNotificationStore((state) => state.clear);
  const hide = () => {
    onClose();
    document.getElementById("notification-center-toggle")?.focus();
  };

  return (
    <section
      id="notification-center"
      className="notification-center"
      aria-label="Notification center"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented) {
          event.preventDefault();
          event.stopPropagation();
          hide();
        }
      }}
    >
      <header className="notification-center-header">
        <h2>Notifications</h2>
        <div className="notification-center-tools">
          <button
            type="button"
            aria-label="Clear all notifications"
            title="Clear all notifications"
            onClick={clear}
            disabled={entries.length === 0}
          >
            <Icon id="clear-all" label="Clear all notifications" />
          </button>
          <button
            type="button"
            aria-label="Hide notification center"
            title="Hide notification center"
            onClick={hide}
          >
            <Icon id="close" label="Hide notification center" />
          </button>
        </div>
      </header>
      <label className="notification-center-dnd">
        <input
          type="checkbox"
          checked={doNotDisturb}
          onChange={(event) => setDoNotDisturb(event.target.checked)}
        />
        Do not disturb
      </label>
      <div
        className="notification-center-list"
        role="log"
        aria-label="Notification history"
        aria-live="off"
      >
        {entries.length === 0 ? (
          <div className="notification-empty">
            <Icon id="bell" size="lg" />
            <p>No notifications</p>
          </div>
        ) : (
          [...entries]
            .reverse()
            .map((entry) => <NotificationContent key={entry.id} entry={entry} compact />)
        )}
      </div>
    </section>
  );
}
