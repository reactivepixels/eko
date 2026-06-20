/**
 * Phase 1 · Region + role vocabulary — the cross-chassis structural contract.
 * See docs/skin-architecture.md §3 (orthogonality + parity rules).
 *
 * REGIONS are the named slots a layout places components into. ROLES are the capabilities a
 * chassis fills (with whatever component fits its language). A layout is portable across two
 * chassis only where they share this vocabulary — so it's deliberately small and stable.
 *
 * Parity is enforced at the ROLE + REGION level, never at component implementations:
 *   - every chassis must fill REQUIRED_ROLES (or it isn't a working player),
 *   - it fills each role with any component bound to that role's feeds (ROLE_FEEDS).
 */

import type { FeedId } from "./feeds";

/** The shared region slots. A chassis may leave regions empty; signature regions (e.g. a
 *  Studio-only flourish) are added per-chassis and simply don't travel to other chassis. */
export type Region =
  | "header" // brand · appearance pickers · navigation
  | "nowPlaying" // cover · title · format badge
  | "tone" // EQ + presets
  | "status" // signal seal · meter
  | "transport" // play / prev / next + seek
  | "library" // browse grid/list
  | "queue" // up-next list
  | "footer" // signal path
  | "dock"; // persistent transport strip (used by Studio's "Library" layout)

export const REGIONS: readonly Region[] = [
  "header",
  "nowPlaying",
  "tone",
  "status",
  "transport",
  "library",
  "queue",
  "footer",
  "dock",
] as const;

/** Capabilities a chassis can surface. The first four are REQUIRED (a complete, honest player). */
export type Role =
  // ── required ──
  | "transport"
  | "seek"
  | "volume"
  | "signalSeal"
  // ── tone ──
  | "eq"
  | "presets"
  // ── meters ──
  | "meter"
  | "spectrum"
  // ── now-playing atoms ──
  | "cover"
  | "trackInfo"
  | "formatBadge"
  // ── library / nav ──
  | "library"
  | "search"
  | "sourceToggle"
  | "queue"
  | "navigation"
  // ── modes ──
  | "repeat"
  | "shuffle"
  | "replayGain"
  | "crossfade"
  | "deviceSelector"
  // ── appearance pickers (palette + chassis are orthogonal to layout) ──
  | "accentPicker"
  | "themeToggle"
  | "chassisPicker";

/** The minimum every chassis must fill — function + the honest seal. */
export const REQUIRED_ROLES: readonly Role[] = [
  "transport",
  "seek",
  "volume",
  "signalSeal",
] as const;

/** Which engine feeds a component filling a role must bind to. Every id here is a real
 *  `FeedId`, so a typo or a renamed feed fails typecheck — the contract can't silently rot. */
export const ROLE_FEEDS: Record<Role, readonly FeedId[]> = {
  transport: ["playPause", "skipNext", "skipPrev"],
  seek: ["position"],
  volume: ["volume"],
  signalSeal: ["bitPerfect", "engineInfo"],

  eq: ["bandGains", "eqBands", "eqEnabled", "preamp"],
  presets: ["eqPreset", "eqPresets"],

  meter: ["spectrum"],
  spectrum: ["spectrum"],

  cover: ["currentTrack"],
  trackInfo: ["currentTrack"],
  formatBadge: ["engineInfo", "currentTrack"],

  library: ["queue", "source", "libSection", "librarySort", "query"],
  search: ["query"],
  sourceToggle: ["source"],
  queue: ["queue"],
  navigation: ["playerView"],

  repeat: ["repeat"],
  shuffle: ["shuffle"],
  replayGain: ["replayGain"],
  crossfade: ["crossfade", "gapless"],
  deviceSelector: ["outputDevice", "engineInfo"],

  accentPicker: ["accent"],
  themeToggle: ["theme"],
  chassisPicker: ["chassis"],
};

/** Is a role one every chassis must provide? */
export function isRequiredRole(role: Role): boolean {
  return REQUIRED_ROLES.includes(role);
}
