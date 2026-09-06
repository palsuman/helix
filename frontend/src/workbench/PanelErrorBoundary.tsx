import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  name: string;
  children: ReactNode;
  failedLabel: string;
  reloadLabel: string;
}

interface State {
  error: Error | null;
  reloadKey: number;
}

/** Isolates one lazy view or panel from the rest of the workbench. */
export class PanelErrorBoundary extends Component<Props, State> {
  state: State = { error: null, reloadKey: 0 };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // The renderer console is captured by the host in packaged builds. Keep
    // panel identity attached so the failure is actionable in diagnostics.
    console.error(`workbench panel '${this.props.name}' crashed`, error, info.componentStack);
  }

  private readonly reload = () => {
    this.setState((state) => ({ error: null, reloadKey: state.reloadKey + 1 }));
  };

  render() {
    if (this.state.error !== null) {
      return (
        <div className="workbench-panel-error" role="alert">
          <strong>{this.props.failedLabel}</strong>
          <span dir="auto">{this.state.error.message}</span>
          <button type="button" onClick={this.reload}>
            {this.props.reloadLabel}
          </button>
        </div>
      );
    }
    return <div key={this.state.reloadKey}>{this.props.children}</div>;
  }
}
