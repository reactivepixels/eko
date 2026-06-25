/**
 * Presets — a named bundle of slot→variant defaults (docs/component-variants-plan.md §4).
 *
 * A preset == the existing `skin` id (porcelain | studio | aether). Selecting a skin (native Skins
 * menu) chooses the preset; each skin then renders only its own slot variants. The cross-skin
 * "Customize" picker was REMOVED — there is no per-slot override layer anymore. The Slot resolver
 * reads: preset default ?? free default.
 *
 * Pure data — only variant id strings + the `Role`/`Skin` types. No component imports.
 */
import type { Role } from "./vocabulary";
import type { Skin } from "../store/useUiStore";

/** The free fallback variant per slot — always a registered free (Porcelain) variant. */
export const DEFAULT_SLOT_VARIANT: Partial<Record<Role, string>> = {
  volume: "porcelainVolume",
  eq: "porcelainEq",
  spectrum: "porcelainSpectrum",
  transport: "porcelainTransport",
  seek: "porcelainSeek",
  meter: "meter",
};

/** Per-preset slot defaults. Porcelain uses the free defaults; Studio/Aether supply their own
 *  control variants. The level meter is the single shared "meter" variant for every preset. */
const PRESET_SLOTS: Partial<Record<Skin, Partial<Record<Role, string>>>> = {
  porcelain: { ...DEFAULT_SLOT_VARIANT },
  studio: {
    volume: "studioVolume",
    eq: "studioEq",
    spectrum: "studioSpectrum",
    transport: "studioTransport",
    seek: "studioSeek",
    meter: "meter",
  },
  aether: {
    volume: "aetherVolume",
    eq: "aetherEq",
    spectrum: "aetherSpectrum",
    transport: "aetherTransport",
    seek: "aetherSeek",
    meter: "meter",
  },
};

export function presetSlots(skin: Skin): Partial<Record<Role, string>> {
  return PRESET_SLOTS[skin] ?? {};
}

/** Presets whose Shell renders slot-composed deck/transport.
 *  Every theme now routes through the generic DeckShell/TransportShell, each rendering only its
 *  own variants (the cross-skin Customize picker was removed). */
export const GENERIC_SHELL_PRESETS = new Set<Skin>(["porcelain", "studio", "aether"]);

