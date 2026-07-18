import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
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

window.addEventListener("error", (event) => showStartupError(event.error || event.message));
window.addEventListener("unhandledrejection", (event) => showStartupError(event.reason));

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
  showStartupError(error);
}
