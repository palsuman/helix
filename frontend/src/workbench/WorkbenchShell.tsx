import { lazy, Suspense, useEffect, useMemo, type CSSProperties, type ReactNode } from "react";
import type { IpcClient } from "../ipc";
import { Icon } from "../icons";
import { useMessage } from "../localization";
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

function IsolatedPanel({ name, children }: { name: string; children: ReactNode }) {
  const t = useMessage();
  return (
    <PanelErrorBoundary
      name={name}
      failedLabel={t("workbenchPanelFailed", { name })}
      reloadLabel={t("workbenchReloadPanel")}
    >
      <Suspense fallback={<p className="workbench-placeholder">{t("workbenchLoading")}</p>}>
        {children}
      </Suspense>
    </PanelErrorBoundary>
  );
}

export function WorkbenchShell({
  client,
  windowId = "main",
  titleBar,
  activities,
  panels,
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
  const t = useMessage();
  const resolvedActivities = useMemo(
    () =>
      activities ?? [
        { id: "explorer", label: t("workbenchExplorer"), icon: "◇", view: <LazyExplorer /> },
        { id: "search", label: t("workbenchSearch"), icon: "⌕", view: <LazyExplorer /> },
        {
          id: "source-control",
          label: t("workbenchSourceControl"),
          icon: "⑂",
          view: <LazyExplorer />,
        },
      ],
    [activities, t],
  );
  const resolvedPanels = useMemo(
    () =>
      panels ?? [
        { id: "problems", label: t("workbenchProblems"), content: <LazyProblems /> },
        { id: "output", label: t("workbenchOutput"), content: <LazyProblems /> },
      ],
    [panels, t],
  );
  const layout = useLayoutStore();
  const persistence = useLayoutPersistence(client, windowId);
  const activeActivity = resolvedActivities.find(
    (activity) => activity.id === layout.activeActivity,
  );
  const activePanel = resolvedPanels.find((panel) => panel.id === layout.activePanel);
  const primaryOnLeft = layout.primarySidebarPosition === "left";

  useEffect(() => {
    if (layout.activeProfile === null) return;
    useLayoutStore.getState().reconcileActiveProfileViews({
      activityIds: resolvedActivities.map((activity) => activity.id),
      panelIds: resolvedPanels.map((panel) => panel.id),
    });
  }, [
    layout.activeActivity,
    layout.activePanel,
    layout.activeProfile,
    resolvedActivities,
    resolvedPanels,
  ]);

  const renderEditor = (groupId: string, index: number) => {
    if (typeof editor === "function") return editor(groupId, index);
    if (index === 0 && editor !== undefined) return editor;
    return (
      <p className="workbench-placeholder">{t("workbenchEditorGroup", { number: index + 1 })}</p>
    );
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
    const sideLabel = side === "left" ? t("workbenchLeftSideLower") : t("workbenchRightSideLower");
    const isPrimarySide = layout.primarySidebarPosition === side;
    const visible = isPrimarySide ? layout.primarySidebarVisible : layout.secondarySidebarVisible;
    const extension =
      side === "left" ? leftActivityRail : (rightActivityRail ?? auxiliaryActivityRail);

    return (
      <nav
        className={`workbench-activity workbench-activity--${side} workbench-frame-surface`}
        aria-label={side === "left" ? t("workbenchLeftRail") : t("workbenchRightRail")}
      >
        {isPrimarySide &&
          resolvedActivities.map((activity) => (
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
              aria-label={t(visible ? "workbenchHidePanel" : "workbenchShowPanel", {
                side: sideLabel,
              })}
              aria-pressed={visible}
              onClick={() => {
                if (isPrimarySide) layout.setPrimarySidebarVisible(!visible);
                else layout.setSecondarySidebarVisible(!visible);
              }}
            >
              <Icon id="panel-side" />
            </button>
            {isPrimarySide && (
              <button
                type="button"
                aria-label={t("workbenchSwapPanels")}
                onClick={() => layout.setPrimarySidebarPosition(primaryOnLeft ? "right" : "left")}
              >
                <Icon id="swap-horizontal" />
              </button>
            )}
            {side === "right" && (
              <button
                type="button"
                aria-label={
                  layout.panelVisible
                    ? t("workbenchMoveBottomPanel")
                    : t("workbenchShowBottomPanel")
                }
                onClick={() => {
                  if (layout.panelVisible) {
                    layout.setPanelPosition(layout.panelPosition === "bottom" ? "right" : "bottom");
                  } else {
                    layout.setPanelVisible(true);
                  }
                }}
              >
                <Icon id="panel-bottom" />
              </button>
            )}
          </>
        )}
      </nav>
    );
  };

  const renderSidePanel = (side: "left" | "right") => {
    const sideLabel = side === "left" ? t("workbenchLeftSide") : t("workbenchRightSide");
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
    const name = t("workbenchPanelName", { side: sideLabel });
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
    const sideLabel = side === "left" ? t("workbenchLeftSideLower") : t("workbenchRightSideLower");
    const isPrimarySide = layout.primarySidebarPosition === side;
    const visible = isPrimarySide ? layout.primarySidebarVisible : layout.secondarySidebarVisible;
    if (!visible) return null;

    return (
      <ResizeHandle
        axis="horizontal"
        label={t("workbenchResizePanel", { side: sideLabel })}
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
    <section className="workbench-card workbench-editor" aria-label={t("workbenchEditorArea")}>
      {showLayoutControls && (
        <div className="workbench-editor-actions">
          <button
            type="button"
            onClick={() => layout.splitEditor("horizontal")}
            disabled={layout.editorGroups.length >= 4}
          >
            {t("workbenchSplitRight")}
          </button>
          <button
            type="button"
            onClick={() => layout.splitEditor("vertical")}
            disabled={layout.editorGroups.length >= 4}
          >
            {t("workbenchSplitDown")}
          </button>
        </div>
      )}
      <div className="workbench-editor-grid" style={editorGridStyle}>
        {layout.editorGroups.map((groupId, index) => (
          <article
            key={groupId}
            className={groupId === layout.activeEditorGroup ? "is-active" : undefined}
            aria-label={t("workbenchEditorGroup", { number: index + 1 })}
            onFocusCapture={() => layout.setActiveEditorGroup(groupId)}
          >
            {layout.editorGroups.length > 1 && (
              <button
                type="button"
                className="workbench-editor-close"
                aria-label={t("workbenchCloseEditorGroup", { number: index + 1 })}
                onClick={() => layout.closeEditorGroup(groupId)}
              >
                <Icon
                  id="close"
                  size="sm"
                  label={t("workbenchCloseEditorGroup", {
                    number: index + 1,
                  })}
                />
              </button>
            )}
            <IsolatedPanel name={t("workbenchEditorGroup", { number: index + 1 })}>
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
      aria-label={t("workbenchPanel")}
      style={
        layout.panelPosition === "bottom"
          ? { height: layout.panelSize }
          : { width: layout.panelSize }
      }
    >
      {activePanel !== undefined && (
        <>
          <div className="workbench-panel-tabs" role="tablist" aria-label={t("workbenchPanelTabs")}>
            {resolvedPanels.map((panel) => (
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
                aria-label={t("workbenchClosePanel")}
                onClick={() => layout.setPanelVisible(false)}
              >
                <Icon id="close" size="sm" label={t("workbenchClosePanel")} />
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
          label={t("workbenchResizeBottomPanel")}
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
          {t(persistence.message, persistence.values)}
        </div>
      )}
      {layout.profileNotice !== null && (
        <div className="workbench-notice" role="status">
          <span>{layout.profileNotice}</span>
          <button type="button" onClick={layout.dismissProfileNotice}>
            {t("commonDismiss")}
          </button>
        </div>
      )}
      <header
        className="workbench-titlebar workbench-frame-surface"
        aria-label={t("workbenchTitleBar")}
        data-tauri-drag-region
      >
        {titleBar}
      </header>
      <div className="workbench-body workbench-frame-surface">
        {leftRegion}
        {central}
        {rightRegion}
      </div>
      <footer
        className="workbench-status workbench-frame-surface"
        aria-label={t("workbenchStatusBar")}
      >
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
