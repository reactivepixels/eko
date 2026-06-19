import { create } from "zustand";

export type Source = "server" | "local";
export type PlayerView = "library" | "deck";
export type LibSection = "albums" | "artists" | "tracks" | "folders" | "playlists";
export type LibSort = "name" | "artist" | "year";
export type Theme = "light" | "dark";

const initialTheme = (): Theme => {
  try {
    return localStorage.getItem("eko.theme") === "dark" ? "dark" : "light";
  } catch {
    return "light";
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
  queueOpen: boolean;
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
  toggleQueue: () => void;
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
  queueOpen: false,
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
    set((s) => {
      const theme: Theme = s.theme === "dark" ? "light" : "dark";
      try {
        localStorage.setItem("eko.theme", theme);
      } catch {
        /* ignore */
      }
      return { theme };
    }),
  toggleQueue: () => set((s) => ({ queueOpen: !s.queueOpen })),
  toggleCompact: () => set((s) => ({ compact: !s.compact })),
}));
