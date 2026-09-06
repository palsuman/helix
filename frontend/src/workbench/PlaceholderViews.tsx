import { useMessage } from "../localization";

export function ExplorerView() {
  const t = useMessage();
  return <p className="workbench-placeholder">{t("workbenchNoFolder")}</p>;
}

export function RightPanelView() {
  const t = useMessage();
  return <p className="workbench-placeholder">{t("workbenchRightPanel")}</p>;
}

export function ProblemsView() {
  const t = useMessage();
  return <p className="workbench-placeholder">{t("workbenchNoProblems")}</p>;
}
