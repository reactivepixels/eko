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
import type { DownloadUrl, Track } from "../types";
import type { MenuItem } from "../player/ContextMenu";

// ── License store stubs ───────────────────────────────────────────────────────

export type LicenseTier = "pro" | "free";
export type LicenseSource = "licensed" | "none";
export interface LicenseStatus {
  tier: LicenseTier;
  source: LicenseSource;
  email: string | null;
}

interface LicenseState extends LicenseStatus {
  loaded: boolean;
  error: string | null;
  activateOpen: boolean;
  loadStatus: () => Promise<void>;
  activate: (key: string) => Promise<boolean>;
  deactivate: () => Promise<void>;
  clearError: () => void;
  openActivate: () => void;
  closeActivate: () => void;
}

export const useLicenseStore = create<LicenseState>(() => ({
  tier: "free" as LicenseTier,
  source: "none" as LicenseSource,
  email: null,
  loaded: true,
  error: null,
  activateOpen: false,
  loadStatus: async () => {},
  activate: async (_key: string) => false,
  deactivate: async () => {},
  clearError: () => {},
  openActivate: () => {},
  closeActivate: () => {},
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
  // Branded `DownloadUrl`, matching `src/pro/useOfflineStore.ts`. This is not cosmetic:
  // `tsconfig.json` points `@pro` at THIS file, so it is the signature `npm run typecheck`
  // actually enforces against every `@pro` consumer in the shared tree. If the brand were
  // dropped here, `cacheTrack(id, track.streamSrcUrl, codec)` would typecheck.
  cacheTrack: (trackId: string, downloadUrl: DownloadUrl, codec: string) => Promise<CacheEntry>;
  cacheAlbum: (trackIds: string[], downloadUrls: DownloadUrl[], codecs: string[]) => Promise<void>;
  removeOffline: (trackId: string) => Promise<void>;
  setCacheLimit: (bytes: number) => Promise<void>;
  listenForProgress: () => Promise<void>;
}

export const useOfflineStore = create<OfflineState>(() => ({
  entries: [],
  stats: null,
  progress: {},
  loaded: true,
  load: async () => {},
  cacheTrack: async (
    _trackId: string,
    _downloadUrl: DownloadUrl,
    _codec: string,
  ): Promise<CacheEntry> => {
    throw new Error("offline cache requires Pro");
  },
  cacheAlbum: async () => {},
  removeOffline: async () => {},
  setCacheLimit: async () => {},
  listenForProgress: async () => {},
}));

// No `useIsDownloading` stub: `src/pro/index.ts` no longer exports it either (its only
// consumer, `OfflineBadge`, lives inside `src/pro/` and imports it directly). The two barrels
// stay in sync.

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

/**
 * Offline context-menu items — **the whole free-build gate for offline caching.**
 *
 * `useQueue.rowMenuItems`, `useLibrary.trackMenuItems` and `useLibrary.albumMenuItems` are
 * free code, and they spread these into the `MenuItem[]` they already build. Returning `[]`
 * here is what makes the free build render no offline items at all, without those builders
 * containing a single Pro concept or license check. `offlineMenu.test.ts` asserts it.
 *
 * The `[]` also has to be a *complete* removal, separator included — which is why the Pro
 * implementations contribute their own leading separator rather than expecting the caller to
 * add one. A caller-side separator would survive this `[]` and leave a stray divider.
 */
export function offlineTrackMenuItems(_track: Track | undefined): MenuItem[] {
  return [];
}
export function offlineAlbumMenuItems(_albumId: string): MenuItem[] {
  return [];
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

export function LicenseModal(): null {
  return null;
}
export function OfflinePanel(): null {
  return null;
}
/**
 * The prop-taking stubs must declare the real component's props, even though they ignore
 * them.
 *
 * `tsconfig.json` maps `@pro` to this file, so these signatures are what the *free* build
 * typechecks every `<OfflineBadge …/>` against. A stub declared `(): null` accepts no
 * attributes at all, so the first consumer to render `<OfflineBadge trackId={…} />` would
 * fail `npm run typecheck` in the free build while compiling fine in Pro — a trap for
 * whoever wires the offline UI up, and one that only shows up in the build they are least
 * likely to be running.
 *
 * The same rule covers non-components, and there it also carries the branded-URL guarantee:
 * `useOfflineStore.cacheTrack` above keeps `downloadUrl` as `DownloadUrl` because the brand
 * is what makes passing a `streamSrcUrl` (which the server may transcode) a compile error,
 * and a stub that widened it to `string` would switch that guarantee off in exactly the
 * build that has no Pro code to catch the mistake later.
 */
export function OfflineBadge(_props: { trackId?: string; className?: string }): null {
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

// ── Visualizer stub (Pro-only PRESETS — Galaxy itself is free, see
//    src/player/visualizer/) ──────────────────────────────────────────────────

/** Free build: the Pro overlay (Cymatics/Murmuration) never renders; Galaxy renders via
 *  src/player/visualizer/VisualizerOverlay instead (PlayerApp.tsx picks by license tier). */
export function VisualizerOverlay(): null {
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
 * Free build: no Pro themes. The registry registers only Porcelain, and `resolveTheme`
 * falls back to it for any other id — so the free build can never reach a Studio Shell.
 */
export type { ThemeDefinition } from "../skin/registry";
import type { ThemeDefinition } from "../skin/registry";
export const proThemes: ThemeDefinition[] = [];

// Free build: no Pro component variants. The registry holds only the free Porcelain variants;
// the Slot resolver falls back to them, so any slot always renders.
export type { VariantDefinition } from "../skin/variants";
import type { VariantDefinition } from "../skin/variants";
export const proVariants: VariantDefinition[] = [];
