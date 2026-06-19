import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { MiniWindow } from "./player/MiniWindow";

const isMini = getCurrentWindow().label === "mini";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isMini ? <MiniWindow /> : <App />}</React.StrictMode>,
);
