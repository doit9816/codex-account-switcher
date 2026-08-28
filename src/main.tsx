import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { logAppError } from "./api/appLog";
import "./styles.css";

function showStartupError(error: unknown) {
  const root = document.getElementById("root");
  if (!root) return;
  const message = error instanceof Error ? error.stack || error.message : String(error);
  root.innerHTML = `
    <main class="startup-error">
      <h1>CodexSwitcher 启动失败</h1>
      <p>前端启动时遇到错误，请把下面的信息发给维护者。</p>
      <pre></pre>
    </main>
  `;
  const pre = root.querySelector("pre");
  if (pre) pre.textContent = message;
}

window.addEventListener("error", (event) => {
  const error = event.error || event.message;
  void logAppError("unhandled_window_error", error);
  showStartupError(error);
});
window.addEventListener("unhandledrejection", (event) => {
  void logAppError("unhandled_promise_rejection", event.reason);
  showStartupError(event.reason);
});

try {
  const root = document.getElementById("root");
  if (!root) {
    throw new Error("Missing #root element");
  }
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
} catch (error) {
  void logAppError("frontend_startup", error);
  showStartupError(error);
}
