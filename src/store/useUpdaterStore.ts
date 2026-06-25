import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { create } from "zustand";

// The in-app auto-updater is DISABLED until release signing is set up. There is no
// Tauri updater private key (TAURI_SIGNING_PRIVATE_KEY) in CI yet, so releases ship no
// `latest.json` and the update endpoint 404s — checking would only surface errors.
// Flip to `true` once the updater key (+ Apple signing) are in place. See docs/RELEASE.md.
export const UPDATER_ENABLED: boolean = false;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type UpdaterPhase = "idle" | "checking" | "available" | "downloading" | "ready" | "error";

interface UpdaterState {
  phase: UpdaterPhase;
  /** Version string of the available update, if any. */
  availableVersion: string | null;
  /** Release notes for the available update, if any. */
  releaseNotes: string | null;
  /** Download progress 0–100 (null if unknown). */
  progress: number | null;
  /** Error message if phase === "error". */
  error: string | null;

  // --- actions ---
  /** Silent background check — called on launch. Does nothing if offline or no update available. */
  checkSilent: () => Promise<void>;
  /** Manual check — shows "checking" and surfaces errors to the UI. */
  checkManual: () => Promise<void>;
  /** Download + install the available update then relaunch. */
  installAndRelaunch: () => Promise<void>;
  /** Dismiss an error or the "available" banner without installing. */
  dismiss: () => void;
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

export const useUpdaterStore = create<UpdaterState>((set, get) => ({
  phase: "idle",
  availableVersion: null,
  releaseNotes: null,
  progress: null,
  error: null,

  checkSilent: async () => {
    if (!UPDATER_ENABLED) return;
    // Don't re-check if already in a non-idle state.
    if (get().phase !== "idle") return;
    try {
      const update = await check();
      if (update) {
        set({
          phase: "available",
          availableVersion: update.version,
          releaseNotes: update.body ?? null,
        });
      }
      // No update → stay idle, say nothing.
    } catch {
      // Offline or endpoint 404 (no latest.json yet) — fail silently.
    }
  },

  checkManual: async () => {
    if (!UPDATER_ENABLED) return;
    if (get().phase === "downloading" || get().phase === "ready") return;
    set({ phase: "checking", error: null, availableVersion: null, releaseNotes: null });
    try {
      const update = await check();
      if (update) {
        set({
          phase: "available",
          availableVersion: update.version,
          releaseNotes: update.body ?? null,
        });
      } else {
        // Up to date — return to idle so the panel shows the "up to date" state.
        set({ phase: "idle" });
      }
    } catch (err) {
      const msg =
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : "Unable to check for updates.";
      set({ phase: "error", error: msg });
    }
  },

  installAndRelaunch: async () => {
    if (!UPDATER_ENABLED) return;
    if (get().phase !== "available") return;
    set({ phase: "downloading", progress: null });
    try {
      const update = await check();
      if (!update) {
        set({ phase: "idle" });
        return;
      }
      let downloaded = 0;
      let total: number | undefined;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength;
          set({ progress: 0 });
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          set({ progress: total ? Math.round((downloaded / total) * 100) : null });
        } else if (event.event === "Finished") {
          set({ phase: "ready", progress: 100 });
        }
      });
      // downloadAndInstall resolves after install; relaunch immediately.
      await relaunch();
    } catch (err) {
      const msg =
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : "Update failed. Please try again.";
      set({ phase: "error", error: msg });
    }
  },

  dismiss: () => set({ phase: "idle", error: null }),
}));
