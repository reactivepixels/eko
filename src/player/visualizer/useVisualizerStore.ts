/**
 * Visualizer overlay state — persisted to localStorage.
 *
 * open       : whether the full-window overlay is visible
 * presetId   : which VisualizerDef.id is active ("galaxy" by default)
 * openWith   : set preset + open in one call
 * close      : hide the overlay (renderer paused)
 * setPreset  : switch preset without changing open state
 * toggle     : toggle open/close with the current preset
 */

import { create } from "zustand";

const LS_KEY = "eko.viz.presetId";

function readPresetId(): string {
  try {
    return localStorage.getItem(LS_KEY) ?? "galaxy";
  } catch {
    return "galaxy";
  }
}

interface VisualizerState {
  open: boolean;
  presetId: string;
  openWith: (id: string) => void;
  close: () => void;
  setPreset: (id: string) => void;
  toggle: () => void;
}

export const useVisualizerStore = create<VisualizerState>((set) => ({
  open: false,
  presetId: readPresetId(),

  openWith: (id: string) => {
    try {
      localStorage.setItem(LS_KEY, id);
    } catch {
      /* ignore */
    }
    set({ open: true, presetId: id });
  },

  close: () => set({ open: false }),

  setPreset: (id: string) => {
    try {
      localStorage.setItem(LS_KEY, id);
    } catch {
      /* ignore */
    }
    set({ presetId: id });
  },

  toggle: () => set((s) => ({ open: !s.open })),
}));
