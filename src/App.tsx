import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen } from "@tauri-apps/api/event";
import { PlayerApp } from "./player/PlayerApp";
import { ConnectPanel } from "./windows/ConnectPanel";
import { ManageServersPanel } from "./windows/ManageServersPanel";
import { useSubsonic } from "./subsonic/useSubsonic";
import { useLocal } from "./local/useLocal";
import { useUiStore } from "./store/useUiStore";
import { usePlayerStore, pauseMainPoll, resumeMainPoll } from "./store/usePlayerStore";
import { useLicenseStore, useOfflineStore, LicenseModal } from "@pro";
import { useUpdaterStore } from "./store/useUpdaterStore";
import { restoreState, startAutosave } from "./store/persist";
import { AUDIO_EXTENSIONS } from "./audio/loader";
import "./App.css";

function App() {
  const source = useUiStore((s) => s.source);
  const connected = useSubsonic((s) => s.connected);
  const compact = useUiStore((s) => s.compact);

  useEffect(() => {
    usePlayerStore.getState().init();
    let stopAutosave = () => {};
    restoreState().finally(() => {
      stopAutosave = startAutosave();
    });

    // Load license status on boot (Pro if a valid key is present, else Free).
    void useLicenseStore.getState().loadStatus();

    // Silent background update check — fails gracefully if offline or no latest.json yet.
    void useUpdaterStore.getState().checkSilent();

    // Offline cache: load the entry list and subscribe to download-progress events.
    void useOfflineStore.getState().load();
    void useOfflineStore.getState().listenForProgress();

    void useSubsonic.getState().autoConnect();
    void useLocal.getState().autoRestore();

    const win = getCurrentWindow();
    const unlistenP = win.onDragDropEvent((event) => {
      if (event.payload.type === "drop") {
        const paths = event.payload.paths.filter((p) =>
          AUDIO_EXTENSIONS.includes(p.split(".").pop()?.toLowerCase() ?? ""),
        );
        if (paths.length) void usePlayerStore.getState().addPaths(paths);
      }
    });

    return () => {
      stopAutosave();
      void unlistenP.then((f) => f());
    };
  }, []);

  // Keyboard shortcuts (main window). Ignored while typing in the search box.
  useEffect(() => {
    let lastVol = 0;
    const onKey = (e: KeyboardEvent) => {
      const el = document.activeElement as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable))
        return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const p = usePlayerStore.getState();
      switch (e.key) {
        case " ":
        case "k":
        case "K":
          e.preventDefault();
          void p.togglePlay();
          break;
        case "ArrowRight":
          e.preventDefault();
          if (e.shiftKey) void p.next();
          else p.seek(Math.min(p.duration, p.currentTime + 5));
          break;
        case "ArrowLeft":
          e.preventDefault();
          if (e.shiftKey) void p.prev();
          else p.seek(Math.max(0, p.currentTime - 5));
          break;
        case "l":
        case "L":
          e.preventDefault();
          p.seek(Math.min(p.duration, p.currentTime + 10));
          break;
        case "j":
        case "J":
          e.preventDefault();
          p.seek(Math.max(0, p.currentTime - 10));
          break;
        case "ArrowUp":
          e.preventDefault();
          p.setVolume(Math.min(1, p.volume + 0.05));
          break;
        case "ArrowDown":
          e.preventDefault();
          p.setVolume(Math.max(0, p.volume - 0.05));
          break;
        case "n":
        case "N":
          e.preventDefault();
          void p.next();
          break;
        case "b":
        case "B":
          e.preventDefault();
          void p.prev();
          break;
        case "m":
        case "M":
          e.preventDefault();
          if (p.volume > 0) {
            lastVol = p.volume;
            p.setVolume(0);
          } else {
            p.setVolume(lastVol || 0.5);
          }
          break;
        default:
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The mini player reads playback state straight from the Rust engine; the main window
  // only needs to handle the actions that require the playlist (next/prev/expand).
  useEffect(() => {
    const unCmd = listen<{ action: string; value?: number }>("eko:cmd", (e) => {
      const { action, value } = e.payload;
      const p = usePlayerStore.getState();
      if (action === "toggle") void p.togglePlay();
      // OS remote commands (media keys / Control Center) send explicit play & pause.
      else if (action === "play") {
        if (!p.isPlaying) void p.togglePlay();
      } else if (action === "pause") {
        if (p.isPlaying) void p.togglePlay();
      } else if (action === "stop") p.stop();
      else if (action === "prev") void p.prev();
      else if (action === "next") void p.next();
      else if (action === "seek" && typeof value === "number") p.seek(value);
      else if (action === "expand") useUiStore.getState().toggleCompact();
    });
    return () => {
      void unCmd.then((f) => f());
    };
  }, []);

  // Show the mini window / hide the main one (and back) when compact toggles.
  // Pause the main poll while in compact mode (the mini window reads engine state
  // directly from Rust); restart it when returning while a track is active.
  useEffect(() => {
    if (compact) {
      pauseMainPoll();
    } else {
      const ps = usePlayerStore.getState();
      if (ps.isPlaying || ps.engineActive) resumeMainPoll();
    }
    void (async () => {
      try {
        const mini = await WebviewWindow.getByLabel("mini");
        const main = getCurrentWindow();
        if (compact) {
          await mini?.show();
          await mini?.setFocus();
          await main.hide();
        } else {
          await mini?.hide();
          await main.show();
        }
      } catch {
        /* ignore */
      }
    })();
  }, [compact]);

  return (
    <>
      <PlayerApp />
      {source === "server" && !connected && <ConnectPanel />}
      <ManageServersPanel />
      <LicenseModal />
    </>
  );
}

export default App;
