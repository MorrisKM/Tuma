import { Component, type ErrorInfo, type ReactNode } from "react";
import { reportError } from "../api";

/** Shows what went wrong instead of an empty window when a view fails to render. */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    reportError("render", `${error.message}\n${error.stack ?? ""}\nIn:${info.componentStack ?? ""}`);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="crashed">
        <h2>Something went wrong in this view</h2>
        <p className="muted">
          Your saved requests are safe. Reloading the window usually fixes it. The details are in crash.log in the
          app's data folder.
        </p>
        <pre className="mono">{error.message}</pre>
        <div className="actions">
          <button className="button primary" onClick={() => location.reload()}>
            Reload the window
          </button>
          <button className="button" onClick={() => this.setState({ error: null })}>
            Try again
          </button>
        </div>
      </div>
    );
  }
}
