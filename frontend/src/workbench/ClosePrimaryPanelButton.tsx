import { Icon } from "../icons";
import { useMessage } from "../localization";
import { useLayoutStore } from "./layoutStore";

export function ClosePrimaryPanelButton() {
  const t = useMessage();
  const side = useLayoutStore((state) => state.primarySidebarPosition);
  const label = t("workbenchHidePanel", {
    side: t(side === "left" ? "workbenchLeftSideLower" : "workbenchRightSideLower"),
  });
  return (
    <button
      type="button"
      className="workbench-view-close"
      aria-label={label}
      title={label}
      onClick={() => {
        const layout = useLayoutStore.getState();
        layout.setPrimarySidebarVisible(false);
        document
          .querySelector<HTMLButtonElement>(
            `.workbench-activity--${side} button[data-active-activity="true"]`,
          )
          ?.focus();
      }}
    >
      <Icon id="close" />
    </button>
  );
}
