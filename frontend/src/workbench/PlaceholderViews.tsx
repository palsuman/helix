import { useMessage } from "../localization";
import { Icon } from "../icons";
import { ClosePrimaryPanelButton } from "./ClosePrimaryPanelButton";

export function ExplorerView() {
  const t = useMessage();
  return (
    <section className="workbench-sidebar-view" aria-label={t("workbenchExplorer")}>
      <header className="workbench-view-header">
        <h2>{t("workbenchExplorer")}</h2>
        <ClosePrimaryPanelButton />
      </header>
      <div className="workbench-view-empty">
        <Icon id="explorer" size="lg" />
        <p>{t("workbenchNoFolder")}</p>
      </div>
    </section>
  );
}

export function SourceControlView() {
  const t = useMessage();
  return (
    <section className="workbench-sidebar-view" aria-label={t("workbenchSourceControl")}>
      <header className="workbench-view-header">
        <h2>{t("workbenchSourceControl")}</h2>
        <ClosePrimaryPanelButton />
      </header>
      <div className="workbench-view-empty">
        <Icon id="source-control" size="lg" />
        <p>{t("workbenchNoFolder")}</p>
      </div>
    </section>
  );
}

export function RightPanelView() {
  const t = useMessage();
  return <p className="workbench-placeholder">{t("workbenchRightPanel")}</p>;
}

export function ProblemsView() {
  const t = useMessage();
  return <p className="workbench-placeholder">{t("workbenchNoProblems")}</p>;
}
