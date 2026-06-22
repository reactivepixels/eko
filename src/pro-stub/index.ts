/**
 * Pro stub barrel — free-build replacements for all Pro exports.
 *
 * Resolved by the `@pro` Vite alias when VITE_PRO is unset (the free build).
 * Every symbol the shared codebase imports from `@pro` must have a stub here
 * so the free build typechecks and compiles even with `src/pro/` physically deleted.
 *
 * Keep this list in sync with `src/pro/index.ts`.
 *
 * Stores are real Zustand stores (so `.getState()` works in App.tsx) but with
 * no-op actions and empty/false state.  Components are null-rendering stubs.
 */

import { create } from "zustand";

// ── License store stubs ───────────────────────────────────────────────────────

export type LicenseTier = "pro" | "trial" | "free";
export type LicenseSource = "licensed" | "trial" | "none";
export interface LicenseStatus {
  tier: LicenseTier;
  source: LicenseSource;
  trialDaysLeft: number | null;
  email: string | null;
}

interface LicenseState extends LicenseStatus {
  loaded: boolean;
  error: string | null;
  loadStatus: () => Promise<void>;
  activate: (key: string) => Promise<void>;
  deactivate: () => Promise<void>;
  clearError: () => void;
}

export const useLicenseStore = create<LicenseState>(() => ({
  tier: "free" as LicenseTier,
  source: "none" as LicenseSource,
  trialDaysLeft: null,
  email: null,
  loaded: true,
  error: null,
  loadStatus: async () => {},
  activate: async (_key: string) => {},
  deactivate: async () => {},
  clearError: () => {},
}));

/** Free build: always false. */
export function useIsPro(): boolean {
  return false;
}

// ── Offline store stubs ───────────────────────────────────────────────────────

export interface CacheEntry {
  trackId: string;
  fileName: string;
  bytes: number;
  codec: string;
  cachedAt: number;
  lastPlayedAt: number;
  partial: boolean;
  transcoded: boolean;
}
export interface CacheStats {
  usedBytes: number;
  capBytes: number;
  trackCount: number;
  transcodeMode: boolean;
}
export interface CacheProgress {
  trackId: string;
  bytesDownloaded: number;
  bytesTotal: number;
  status: string;
  error?: string;
}

interface OfflineState {
  entries: CacheEntry[];
  stats: CacheStats | null;
  progress: Record<string, CacheProgress>;
  loaded: boolean;
  load: () => Promise<void>;
  cacheTrack: (trackId: string, url: string, codec: string) => Promise<CacheEntry>;
  cacheAlbum: (trackIds: string[], downloadUrls: string[], codecs: string[]) => Promise<void>;
  removeOffline: (trackId: string) => Promise<void>;
  setCacheLimit: (bytes: number) => Promise<void>;
  setCacheBitrate: (transcode: boolean) => Promise<void>;
  listenForProgress: () => Promise<void>;
}

export const useOfflineStore = create<OfflineState>(() => ({
  entries: [],
  stats: null,
  progress: {},
  loaded: true,
  load: async () => {},
  cacheTrack: async (_trackId: string, _url: string, _codec: string): Promise<CacheEntry> => {
    throw new Error("offline cache requires Pro");
  },
  cacheAlbum: async () => {},
  removeOffline: async () => {},
  setCacheLimit: async () => {},
  setCacheBitrate: async () => {},
  listenForProgress: async () => {},
}));

/** Free build: never downloading. */
export function useIsDownloading(_trackId?: string): boolean {
  return false;
}
/** Free build: nothing is offline. */
export function isOffline(_entries: CacheEntry[], _trackId: string | undefined): boolean {
  return false;
}
/** Free build: no offline entry. */
export function offlineEntry(
  _entries: CacheEntry[],
  _trackId: string | undefined,
): CacheEntry | undefined {
  return undefined;
}

// ── Smart playlist stubs ──────────────────────────────────────────────────────

interface SmartPlaylistState {
  defs: never[];
  editing: null;
  loading: Record<string, boolean>;
  error: Record<string, string | null>;
  mixLoading: boolean;
  mixError: string | null;
  createDef: () => void;
  updateEditing: () => void;
  addRule: () => void;
  removeRule: () => void;
  updateRule: () => void;
  saveEditing: () => void;
  cancelEditing: () => void;
  openForEdit: () => void;
  deleteDef: () => void;
  play: () => Promise<void>;
  previewCount: () => Promise<number>;
  instantMixFromTrack: (trackId: string, genre?: string) => Promise<void>;
  instantMixFromArtist: (artistId: string, genre?: string) => Promise<void>;
}

export const useSmartPlaylistStore = create<SmartPlaylistState>(() => ({
  defs: [],
  editing: null,
  loading: {},
  error: {},
  mixLoading: false,
  mixError: null,
  createDef: () => {},
  updateEditing: () => {},
  addRule: () => {},
  removeRule: () => {},
  updateRule: () => {},
  saveEditing: () => {},
  cancelEditing: () => {},
  openForEdit: () => {},
  deleteDef: () => {},
  play: async () => {},
  previewCount: async () => 0,
  instantMixFromTrack: async (_trackId: string, _genre?: string) => {},
  instantMixFromArtist: async (_artistId: string, _genre?: string) => {},
}));

export async function evaluateSmartPlaylist(_def: unknown): Promise<never[]> {
  return [];
}
export async function buildInstantMix(
  _trackId: string,
  _genre?: string,
  _targetCount?: number,
): Promise<never[]> {
  return [];
}
export async function buildArtistMix(
  _artistId: string,
  _genre?: string,
  _targetCount?: number,
): Promise<never[]> {
  return [];
}

// ── Component stubs (all render null in the free build) ───────────────────────

export function ProPanel(): null {
  return null;
}
export function OfflinePanel(): null {
  return null;
}
export function OfflineBadge(): null {
  return null;
}
export function OfflineAction(): null {
  return null;
}
export function OfflineView(): null {
  return null;
}
export function DownloadProgressBar(): null {
  return null;
}
export function ParametricEqPanel(): null {
  return null;
}
export function SmartPlaylistsView(): null {
  return null;
}

// ── Theme / skin stubs (Pro features — locked out in free build) ──────────────

/** Free build: no-op — native Skins menu is not registered in the free build. */
export function useNativeMenu(): void {
  // intentional no-op: Skins menu does not exist in the free build
}

// Note: ThemeSwitcher (light/dark toggle) is FREE — it lives in src/player/,
// not behind @pro. No stub needed here.

/**
 * Free build: always renders the Porcelain children.
 * The Studio skin is Pro-only; the `children` prop carries the Porcelain layout.
 */
import type { ReactNode } from "react";
export function StudioApp({ children }: { children: ReactNode }): ReactNode {
  return children;
}
