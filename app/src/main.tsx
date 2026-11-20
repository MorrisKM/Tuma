import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { reportError } from "./api";

window.addEventListener("error", (e) => reportError("error", e.error ?? e.message));
window.addEventListener("unhandledrejection", (e) => reportError("unhandled rejection", e.reason));

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
