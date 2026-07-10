import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { MiniWindow } from "./player/MiniWindow";
import { installRoundRectPolyfill } from "./lib/roundRectPolyfill";
// Populate the theme + component-variant registries before first render (variants are the Pro
// themes' rendering plumbing; the user-facing customization picker is tracked separately — issue #1).
import "./skin/registerThemes";
import "./skin/registerVariants";

// Must run before any canvas draws (Spectrum/Meter) so older macOS WebKit (< 13.3) doesn't
// throw on the missing CanvasRenderingContext2D.roundRect.
installRoundRectPolyfill();

const isMini = getCurrentWindow().label === "mini";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isMini ? <MiniWindow /> : <App />}</React.StrictMode>,
);
