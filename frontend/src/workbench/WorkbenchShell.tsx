import { lazy, Suspense, useEffect, useRef, type CSSProperties, type ReactNode } from "react";
import type { IpcClient } from "../ipc";
import { PanelErrorBoundary } from "./PanelErrorBoundary";
import {
  PANEL_MAX,
  PANEL_MIN,
  PRIMARY_SIDEBAR_MAX,
  PRIMARY_SIDEBAR_MIN,
  SECONDARY_SIDEBAR_MAX,
  SECONDARY_SIDEBAR_MIN,
  useLayoutStore,
} from "./layoutStore";
import { ResizeHandle } from "./ResizeHandle";
import { executeLayoutProfileCommand } from "./profileCommands";
import { useLayoutPersistence } from "./useLayoutPersistence";
import "./workbench.css";

const LazyExplorer = lazy(() =>
  import("./PlaceholderViews").then((module) => ({ default: module.ExplorerView })),
);
const LazyRightPanel = lazy(() =>
  import("./PlaceholderViews").then((module) => ({ default: module.RightPanelView })),
);
const LazyProblems = lazy(() =>
  import("./PlaceholderViews").then((module) => ({ default: module.ProblemsView })),
);

export interface ActivityRegistration {
  id: string;
  label: string;
  icon: ReactNode;
  view: ReactNode;
}

export interface PanelRegistration {
  id: string;
  label: string;
  content: ReactNode;
}

export interface StatusRegistration {
  id: string;
  content: ReactNode;
}

interface WorkbenchShellProps {
  client: IpcClient;
  /** Host window id used to scope layout persistence (REQ-ARCH-006). */
  windowId?: string;
  titleBar?: ReactNode;
  activities?: readonly ActivityRegistration[];
  panels?: readonly PanelRegistration[];
  leftPanel?: ReactNode;
  rightPanel?: ReactNode;
  leftActivityRail?: ReactNode;
  rightActivityRail?: ReactNode;
  /** @deprecated Use rightActivityRail. */
  auxiliaryActivityRail?: ReactNode;
  editor?: ReactNode | ((groupId: string, index: number) => ReactNode);
  statusLeft?: readonly StatusRegistration[];
  statusRight?: readonly StatusRegistration[];
  showLayoutControls?: boolean;
  overlay?: ReactNode;
}

const defaultActivities: readonly ActivityRegistration[] = [
  { id: "explorer", label: "Explorer", icon: "◇", view: <LazyExplorer /> },
  { id: "search", label: "Search", icon: "⌕", view: <LazyExplorer /> },
  { id: "source-control", label: "Source Control", icon: "⑂", view: <LazyExplorer /> },
];

const defaultPanels: readonly PanelRegistration[] = [
  { id: "problems", label: "Problems", content: <LazyProblems /> },
  { id: "output", label: "Output", content: <LazyProblems /> },
];

const loading = <p className="workbench-placeholder">Loading…</p>;

function IsolatedPanel({ name, children }: { name: string; children: ReactNode }) {
  return (
    <PanelErrorBoundary name={name}>
      <Suspense fallback={loading}>{children}</Suspense>
    </PanelErrorBoundary>
  );
}

export function WorkbenchShell({
  client,
  windowId = "main",
  titleBar,
  activities = defaultActivities,
  panels = defaultPanels,
  leftPanel,
  rightPanel,
  leftActivityRail,
  rightActivityRail,
  auxiliaryActivityRail,
  editor,
  statusLeft = [],
  statusRight = [],
  showLayoutControls = true,
  overlay,
}: WorkbenchShellProps) {
  const layout = useLayoutStore();
  const persistence = useLayoutPersistence(client, windowId);
  const zenChordStartedAt = useRef(0);
  const activeActivity = activities.find((activity) => activity.id === layout.activeActivity);
  const activePanel = panels.find((panel) => panel.id === layout.activePanel);
  const primaryOnLeft = layout.primarySidebarPosition === "left";

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const now = performance.now();
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        zenChordStartedAt.current = now;
        event.preventDefault();
        return;
      }
      if (
        event.key.toLowerCase() === "z" &&
        zenChordStartedAt.current > 0 &&
        now - zenChordStartedAt.current <= 1_500
      ) {
        zenChordStartedAt.current = 0;
        event.preventDefault();
        executeLayoutProfileCommand("workbench.action.toggleZenMode");
        return;
      }
      if (!event.ctrlKey && !event.metaKey && !event.altKey && event.key !== "Shift") {
        zenChordStartedAt.current = 0;
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  useEffect(() => {
    if (layout.activeProfile === null) return;
    useLayoutStore.getState().reconcileActiveProfileViews({
      activityIds: activities.map((activity) => activity.id),
      panelIds: panels.map((panel) => panel.id),
    });
  }, [activities, layout.activeActivity, layout.activePanel, layout.activeProfile, panels]);

  const renderEditor = (groupId: string, index: number) => {
    if (typeof editor === "function") return editor(groupId, index);
    if (index === 0 && editor !== undefined) return editor;
    return <p className="workbench-placeholder">Editor group {index + 1}</p>;
  };

  const editorGridStyle = {
    gridTemplateColumns:
      layout.splitDirection === "horizontal"
        ? `repeat(${layout.editorGroups.length}, minmax(0, 1fr))`
        : "minmax(0, 1fr)",
    gridTemplateRows:
      layout.splitDirection === "vertical"
        ? `repeat(${layout.editorGroups.length}, minmax(0, 1fr))`
        : "minmax(0, 1fr)",
  } satisfies CSSProperties;

  const renderActivityRail = (side: "left" | "right") => {
    const isPrimarySide = layout.primarySidebarPosition === side;
    const visible = isPrimarySide ? layout.primarySidebarVisible : layout.secondarySidebarVisible;
    const extension =
      side === "left" ? leftActivityRail : (rightActivityRail ?? auxiliaryActivityRail);

    return (
      <nav
        className={`workbench-activity workbench-activity--${side} workbench-frame-surface`}
        aria-label={`${side === "left" ? "Left" : "Right"} activity rail`}
      >
        {isPrimarySide &&
          activities.map((activity) => (
            <button
              type="button"
              key={activity.id}
              className={activity.id === activeActivity?.id ? "is-active" : undefined}
              aria-label={activity.label}
              aria-pressed={activity.id === activeActivity?.id}
              onClick={() => layout.setActiveActivity(activity.id)}
            >
              <span aria-hidden="true">{activity.icon}</span>
            </button>
          ))}
        {extension}
        {showLayoutControls && (
          <>
            <div className="workbench-activity-spacer" />
            <button
              type="button"
              aria-label={`${visible ? "Hide" : "Show"} ${side} panel`}
              aria-pressed={visible}
              onClick={() => {
                if (isPrimarySide) layout.setPrimarySidebarVisible(!visible);
                else layout.setSecondarySidebarVisible(!visible);
              }}
            >
              <span aria-hidden="true">▯</span>
            </button>
            {isPrimarySide && (
              <button
                type="button"
                aria-label="Swap left and right panels"
                onClick={() => layout.setPrimarySidebarPosition(primaryOnLeft ? "right" : "left")}
              >
                <span aria-hidden="true">⇄</span>
              </button>
            )}
            {side === "right" && (
              <button
                type="button"
                aria-label={layout.panelVisible ? "Move bottom panel" : "Show bottom panel"}
                onClick={() => {
                  if (layout.panelVisible) {
                    layout.setPanelPosition(layout.panelPosition === "bottom" ? "right" : "bottom");
                  } else {
                    layout.setPanelVisible(true);
                  }
                }}
              >
                <span aria-hidden="true">▤</span>
              </button>
            )}
          </>
        )}
      </nav>
    );
  };

  const renderSidePanel = (side: "left" | "right") => {
    const isPrimarySide = layout.primarySidebarPosition === side;
    const visible = isPrimarySide ? layout.primarySidebarVisible : layout.secondarySidebarVisible;
    if (!visible) return null;

    const fixedContent = side === "left" ? leftPanel : rightPanel;
    const content =
      fixedContent !== undefined ? (
        fixedContent
      ) : isPrimarySide ? (
        activeActivity?.view
      ) : (
        <LazyRightPanel />
      );
    const name = `${side === "left" ? "Left" : "Right"} panel`;
    return (
      <aside
        className={`workbench-card workbench-sidebar workbench-sidebar--${side}`}
        aria-label={name}
        style={{
          width: isPrimarySide ? layout.primarySidebarSize : layout.secondarySidebarSize,
        }}
      >
        {content !== undefined && <IsolatedPanel name={name}>{content}</IsolatedPanel>}
      </aside>
    );
  };

  const renderSideResizeHandle = (side: "left" | "right") => {
    const isPrimarySide = layout.primarySidebarPosition === side;
    const visible = isPrimarySide ? layout.primarySidebarVisible : layout.secondarySidebarVisible;
    if (!visible) return null;

    return (
      <ResizeHandle
        axis="horizontal"
        label={`Resize ${side} panel`}
        value={isPrimarySide ? layout.primarySidebarSize : layout.secondarySidebarSize}
        min={isPrimarySide ? PRIMARY_SIDEBAR_MIN : SECONDARY_SIDEBAR_MIN}
        max={isPrimarySide ? PRIMARY_SIDEBAR_MAX : SECONDARY_SIDEBAR_MAX}
        direction={side === "left" ? 1 : -1}
        onChange={isPrimarySide ? layout.setPrimarySidebarSize : layout.setSecondarySidebarSize}
      />
    );
  };

  const leftRegion = (
    <div className="workbench-side-region workbench-side-region--left">
      {renderActivityRail("left")}
      {renderSidePanel("left")}
      {renderSideResizeHandle("left")}
    </div>
  );

  const rightRegion = (
    <div className="workbench-side-region workbench-side-region--right">
      {renderSideResizeHandle("right")}
      {renderSidePanel("right")}
      {renderActivityRail("right")}
    </div>
  );

  const editorArea = (
    <section className="workbench-card workbench-editor" aria-label="Editor area">
      {showLayoutControls && (
        <div className="workbench-editor-actions">
          <button
            type="button"
            onClick={() => layout.splitEditor("horizontal")}
            disabled={layout.editorGroups.length >= 4}
          >
            Split right
          </button>
          <button
            type="button"
            onClick={() => layout.splitEditor("vertical")}
            disabled={layout.editorGroups.length >= 4}
          >
            Split down
          </button>
        </div>
      )}
      <div className="workbench-editor-grid" style={editorGridStyle}>
        {layout.editorGroups.map((groupId, index) => (
          <article
            key={groupId}
            className={groupId === layout.activeEditorGroup ? "is-active" : undefined}
            aria-label={`Editor group ${index + 1}`}
            onFocusCapture={() => layout.setActiveEditorGroup(groupId)}
          >
            {layout.editorGroups.length > 1 && (
              <button
                type="button"
                className="workbench-editor-close"
                aria-label={`Close editor group ${index + 1}`}
                onClick={() => layout.closeEditorGroup(groupId)}
              >
                ×
              </button>
            )}
            <IsolatedPanel name={`Editor group ${index + 1}`}>
              {renderEditor(groupId, index)}
            </IsolatedPanel>
          </article>
        ))}
      </div>
    </section>
  );

  const panelArea = layout.panelVisible && (
    <section
      className="workbench-card workbench-panel"
      aria-label="Panel"
      style={
        layout.panelPosition === "bottom"
          ? { height: layout.panelSize }
          : { width: layout.panelSize }
      }
    >
      {activePanel !== undefined && (
        <>
          <div className="workbench-panel-tabs" role="tablist" aria-label="Panel tabs">
            {panels.map((panel) => (
              <button
                type="button"
                role="tab"
                aria-selected={panel.id === activePanel.id}
                key={panel.id}
                onClick={() => layout.setActivePanel(panel.id)}
              >
                {panel.label}
              </button>
            ))}
            {showLayoutControls && (
              <button
                type="button"
                className="workbench-panel-close"
                aria-label="Close panel"
                onClick={() => layout.setPanelVisible(false)}
              >
                ×
              </button>
            )}
          </div>
          <div role="tabpanel" className="workbench-panel-content">
            <IsolatedPanel name={activePanel.label}>{activePanel.content}</IsolatedPanel>
          </div>
        </>
      )}
    </section>
  );

  const central = (
    <div
      className={`workbench-central workbench-central--panel-${layout.panelPosition}`}
      data-testid="workbench-central"
      style={{ "--workbench-panel-size": `${layout.panelSize}px` } as CSSProperties}
    >
      {editorArea}
      {layout.panelVisible && (
        <ResizeHandle
          axis={layout.panelPosition === "bottom" ? "vertical" : "horizontal"}
          label="Resize panel"
          value={layout.panelSize}
          min={PANEL_MIN}
          max={PANEL_MAX}
          direction={-1}
          onChange={layout.setPanelSize}
        />
      )}
      {panelArea}
    </div>
  );

  return (
    <main className={`workbench${layout.zenMode ? " workbench--zen" : ""}`} data-testid="workbench">
      {overlay}
      {(persistence.kind === "reset" || persistence.kind === "unavailable") && (
        <div className="workbench-notice" role="status">
          {persistence.message}
        </div>
      )}
      {layout.profileNotice !== null && (
        <div className="workbench-notice" role="status">
          <span>{layout.profileNotice}</span>
          <button type="button" onClick={layout.dismissProfileNotice}>
            Dismiss
          </button>
        </div>
      )}
      <header
        className="workbench-titlebar workbench-frame-surface"
        aria-label="Title bar"
        data-tauri-drag-region
      >
        {titleBar}
      </header>
      <div className="workbench-body workbench-frame-surface">
        {leftRegion}
        {central}
        {rightRegion}
      </div>
      <footer className="workbench-status workbench-frame-surface" aria-label="Status bar">
        <div>
          {statusLeft.map((item) => (
            <span key={item.id}>{item.content}</span>
          ))}
        </div>
        <div>
          {statusRight.map((item) => (
            <span key={item.id}>{item.content}</span>
          ))}
        </div>
      </footer>
    </main>
  );
}
