import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/atkinson-hyperlegible-next/400.css";
import "@fontsource/atkinson-hyperlegible-next/500.css";
import "@fontsource/atkinson-hyperlegible-next/700.css";
import App from "./App";
import "./styles.css";

// Dev harness: run the renderer in a plain browser with a mocked backend.
// Never active in production builds or inside the Tauri shell.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  const { installMockBackend } = await import("./dev/mockBackend");
  installMockBackend();
}

const root = document.getElementById("root");
if (!root) throw new Error("Desk: index.html has no #root element");
ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
