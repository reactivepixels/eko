/**
 * Theme registry — the Phase-3 spine (docs/skin-registry-plan.md §2.1).
 *
 * A **theme** is a registered `{ id, label, tier, Shell }`. The Shell renders the entire main
 * window for that theme (top bar → transport) and reads the shared headless hooks itself; it
 * owns all of its pixels (docs/skin-architecture.md §0). Adding a theme is a registration —
 * never an edit to a switch.
 *
 * This module is **pure**: it imports only React *types* and the `Skin` *type*. It pulls in no
 * store, no hook, and no component, so nothing in the app can form an import cycle through it,
 * and it is safe in the free build (which registers only Porcelain).
 *
 * NOTE (deliberate boundary): a theme registers a Shell, NOT a per-slot component map. The
 * `Registry`/`ChassisManifest`/`LayoutDoc` schema in `src/skin/types.ts` stays frozen-as-intent
 * until a third theme reveals what actually varies (docs/skin-architecture.md §4, §9). The
 * optional `slots` field below reserves that future without building it now.
 */
import type { ComponentType } from "react";
import type { Skin } from "../store/useUiStore"; // type-only — does not create a runtime edge

export type ThemeTier = "free" | "pro";

export interface ThemeDefinition {
  /** Matches the `Skin` id stamped as `data-skin` and persisted by `useUiStore`. */
  id: Skin;
  /** Human label for pickers/menus. */
  label: string;
  /** Gating tier. Only Porcelain is `free`; alternate skins are `pro`. */
  tier: ThemeTier;
  /** Renders the whole main window for this theme. Reads hooks itself; owns all pixels. */
  Shell: ComponentType;
  /** Reserved for the Phase-5 per-slot resolver. Unused today — do not build against it. */
  slots?: Readonly<Record<string, ComponentType>>;
}

/** The default + only free theme. Used as the resolve fallback. */
export const DEFAULT_THEME: Skin = "porcelain";

const THEMES = new Map<Skin, ThemeDefinition>();

/** Register a theme. Throws on a duplicate id so a wiring mistake fails loudly, not silently. */
export function registerTheme(def: ThemeDefinition): void {
  if (THEMES.has(def.id)) {
    throw new Error(`Theme "${def.id}" is already registered`);
  }
  THEMES.set(def.id, def);
}

/** The theme for an id, or `undefined` if it isn't registered (e.g. a Pro skin in a free build). */
export function getTheme(id: Skin): ThemeDefinition | undefined {
  return THEMES.get(id);
}

/** All registered themes, in registration order (Porcelain first). For pickers/menus. */
export function listThemes(): ThemeDefinition[] {
  return [...THEMES.values()];
}

/**
 * Render-safe resolver: an unknown/unregistered id (e.g. a stale persisted `studio` pref in a
 * free build) falls back to Porcelain, so the host can always render *something*. Porcelain is
 * always registered before any render (App.tsx imports the registration bootstrap first).
 */
export function resolveTheme(id: Skin): ThemeDefinition {
  return THEMES.get(id) ?? THEMES.get(DEFAULT_THEME)!;
}
