import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { MiniWindow } from "./player/MiniWindow";
// Populate the theme + component-variant registries before first render (variants are the Pro
// themes' rendering plumbing; the user-facing customization picker is tracked separately — issue #1).
import "./skin/registerThemes";
import "./skin/registerVariants";

const isMini = getCurrentWindow().label === "mini";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isMini ? <MiniWindow /> : <App />}</React.StrictMode>,
);
