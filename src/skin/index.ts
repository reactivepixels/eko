/**
 * Skin architecture — Phase 1 contracts (the stable spine).
 * docs/skin-architecture.md §3 (Palette / Chassis / Layout), §4 (primitives), §11 (manifest).
 *
 *   feeds.ts       — the feed catalog (engine/state bindings). Parity lives here.
 *   vocabulary.ts  — regions + roles (the cross-chassis structural contract).
 *   types.ts       — Registry / ChassisManifest / LayoutDoc / UserLayout schema + validators.
 *
 * No UI and no audio-path code lives here — these are contracts the Phase-2 primitives,
 * Phase-3 renderer and Phase-7 builder are written against.
 */
export * from "./feeds";
export * from "./vocabulary";
export * from "./types";
