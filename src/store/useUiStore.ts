import { create } from "zustand";
// @pro resolves to src/pro-stub in the free build (license store returns the free tier),
// so this import is safe there and gates Pro skins by the RUNTIME license in the Pro build.
import { useLicenseStore } from "@pro";

// Free build: light/dark toggle is FREE (Porcelain ↔ Graphite).
// Only alternate SKINS (Studio etc.) are Pro-gated.
// Pro build (VITE_PRO=1) also enables the skin switcher and Studio skin.
const IS_PRO = Boolean(import.meta.env.VITE_PRO);

export type Source = "server" | "local";
export type PlayerView = "library" | "deck";
export type LibSection =
  | "albums"
  | "artists"
  | "tracks"
  | "folders"
  | "playlists"
  | "offline"
  | "smart-playlists";
export type LibSort = "name" | "artist" | "year";
export type Theme = "light" | "dark";
export type Accent = "orange" | "violet" | "blue" | "teal" | "graphite" | "cyan";

/** Built-in accent presets. `swatch` is the picker dot; the actual token
 *  values live in neu.css under `[data-accent="…"]`. Keep the two in sync. */
export const ACCENTS: { id: Accent; label: string; swatch: string }[] = [
  { id: "orange", label: "EKO Orange", swatch: "#ef6a1e" },
  { id: "violet", label: "Violet", swatch: "#6a5cf0" },
  { id: "blue", label: "Blue", swatch: "#2f8fff" },
  { id: "teal", label: "Teal", swatch: "#13b5a6" },
  { id: "graphite", label: "Graphite", swatch: "#8a8780" },
  { id: "cyan", label: "Cyan", swatch: "#28d8f7" },
];

const initialTheme = (): Theme => {
  // Light/dark toggle is FREE — restore persisted preference in both free and Pro builds.
  try {
    return localStorage.getItem("eko.theme") === "dark" ? "dark" : "light";
  } catch {
    return "light";
  }
};

const initialAccent = (): Accent => {
  try {
    const v = localStorage.getItem("eko.accent") as Accent | null;
    return v && ACCENTS.some((a) => a.id === v) ? v : "orange";
  } catch {
    return "orange";
  }
};

export type Skin = "porcelain" | "studio" | "aether";

/** Built-in skins. Each is a token bundle in neu.css under `[data-skin="…"]`
 *  (theme-aware: it may also override `[data-skin="…"][data-theme="dark"]`).
 *  `porcelain` is the default and needs no block (it IS :root). */
export const SKINS: { id: Skin; label: string }[] = [
  { id: "porcelain", label: "Porcelain" },
  { id: "studio", label: "Studio" },
  { id: "aether", label: "Aether" },
];

/** Porcelain is the only free skin; every alternate skin is Pro-gated. The theme registry
 *  carries the same `tier` per theme — this is the store-side gate, kept independent of the
 *  registry (which the free build may delete with `src/pro/`). */
export const isProSkin = (id: Skin): boolean => id !== "porcelain";

const initialSkin = (): Skin => {
  // Free build is always locked to Porcelain. Never restore a persisted Studio choice.
  if (!IS_PRO) return "porcelain";
  try {
    const v = localStorage.getItem("eko.skin") as Skin | null;
    return v && SKINS.some((s) => s.id === v) ? v : "porcelain";
  } catch {
    return "porcelain";
  }
};

interface UiState {
  zoom: number; // 1 | 2 | 3 (double-size etc.)
  mainShade: boolean; // collapsed windowshade mode for the main window
  eqVisible: boolean;
  plVisible: boolean;
  alwaysOnTop: boolean;
  presetsOpen: boolean;

  // --- new player UI ---
  source: Source;
  playerView: PlayerView;
  libSection: LibSection;
  librarySort: LibSort;
  query: string;
  theme: Theme;
  accent: Accent;
  skin: Skin;
  queueOpen: boolean;
  lyricsOpen: boolean;
  compact: boolean;

  setZoom: (z: number) => void;
  toggleMainShade: () => void;
  toggleEq: () => void;
  togglePl: () => void;
  toggleAlwaysOnTop: () => void;
  setPresetsOpen: (v: boolean) => void;
  setSource: (s: Source) => void;
  setPlayerView: (v: PlayerView) => void;
  setLibSection: (s: LibSection) => void;
  setLibrarySort: (s: LibSort) => void;
  setQuery: (q: string) => void;
  toggleTheme: () => void;
  setAccent: (a: Accent) => void;
  setSkin: (s: Skin) => void;
  toggleQueue: () => void;
  toggleLyrics: () => void;
  toggleCompact: () => void;
}

export const useUiStore = create<UiState>((set) => ({
  zoom: 2,
  mainShade: false,
  eqVisible: true,
  plVisible: true,
  alwaysOnTop: false,
  presetsOpen: false,
  source: "server",
  playerView: "library",
  libSection: "albums",
  librarySort: "artist",
  query: "",
  theme: initialTheme(),
  accent: initialAccent(),
  skin: initialSkin(),
  queueOpen: false,
  lyricsOpen: false,
  compact: false,

  // Zoom in 0.25 steps from 1× to 2× (snapped to the nearest quarter).
  setZoom: (z) => set({ zoom: Math.min(2, Math.max(1, Math.round(z * 4) / 4)) }),
  toggleMainShade: () => set((s) => ({ mainShade: !s.mainShade })),
  toggleEq: () => set((s) => ({ eqVisible: !s.eqVisible })),
  togglePl: () => set((s) => ({ plVisible: !s.plVisible })),
  toggleAlwaysOnTop: () => set((s) => ({ alwaysOnTop: !s.alwaysOnTop })),
  setPresetsOpen: (v) => set({ presetsOpen: v }),
  setSource: (s) => set({ source: s }),
  setPlayerView: (v) => set({ playerView: v }),
  setLibSection: (s) => set({ libSection: s }),
  setLibrarySort: (s) => set({ librarySort: s }),
  setQuery: (q) => set({ query: q }),
  toggleTheme: () =>
    // Light/dark toggle is FREE — works in both free and Pro builds.
    set((s) => {
      const theme: Theme = s.theme === "dark" ? "light" : "dark";
      try {
        localStorage.setItem("eko.theme", theme);
      } catch {
        /* ignore */
      }
      return { theme };
    }),
  setAccent: (accent) =>
    set(() => {
      try {
        localStorage.setItem("eko.accent", accent);
      } catch {
        /* ignore */
      }
      return { accent };
    }),
  setSkin: (skin) => {
    // Pro skins require Pro at RUNTIME, not just the build flag: the free build is gated by
    // IS_PRO; the product build additionally rejects the selection when the license tier is
    // "free" (no active trial/license), so a JS patch to the UI gate isn't enough.
    if (isProSkin(skin) && (!IS_PRO || useLicenseStore.getState().tier === "free")) return;
    set(() => {
      try {
        localStorage.setItem("eko.skin", skin);
      } catch {
        /* ignore */
      }
      return { skin };
    });
  },
  toggleQueue: () => set((s) => ({ queueOpen: !s.queueOpen })),
  toggleLyrics: () => set((s) => ({ lyricsOpen: !s.lyricsOpen })),
  toggleCompact: () => set((s) => ({ compact: !s.compact })),
}));

// One-time cleanup: drop any persisted per-slot overrides from the removed Customize picker, so a
// stale cross-skin variant can't keep breaking the deck on launch.
try {
  localStorage.removeItem("eko.slotVariants");
} catch {
  /* ignore */
}
