/**
 * Variant registration bootstrap (docs/component-variants-plan.md §7 Phase A).
 *
 * Explicit composition: free (Porcelain) variants are built here; Pro variants come from `@pro`'s
 * `proVariants` (an empty array in the free-build stub). Variant objects are assembled at this
 * boundary so the variant component files stay pure components (clean Fast-Refresh). Imported once
 * for its side effect by App.tsx, before first render.
 */
import { registerVariant, type VariantDefinition } from "./variants";
import {
  PorcelainVolume,
  PorcelainEq,
  PorcelainSpectrum,
  PorcelainTransport,
  PorcelainSeek,
} from "../player/variants/porcelain";
import { Meter } from "../player/Meter";
import { MiniEq } from "../player/MiniEq";
import { proVariants } from "@pro";

const freeVariants: VariantDefinition[] = [
  {
    id: "porcelainVolume",
    slot: "volume",
    label: "Porcelain · Dial",
    tier: "free",
    Component: PorcelainVolume,
  },
  {
    id: "porcelainEq",
    slot: "eq",
    label: "Porcelain · Faders",
    tier: "free",
    Component: PorcelainEq,
  },
  {
    id: "porcelainSpectrum",
    slot: "spectrum",
    label: "Porcelain · Spectrum",
    tier: "free",
    Component: PorcelainSpectrum,
  },
  {
    id: "porcelainTransport",
    slot: "transport",
    label: "Porcelain · Buttons",
    tier: "free",
    Component: PorcelainTransport,
  },
  {
    id: "porcelainSeek",
    slot: "seek",
    label: "Porcelain · Seek",
    tier: "free",
    Component: PorcelainSeek,
  },
  // The level meter is ONE unified design for every theme (material from tokens) — a single shared
  // variant fills the meter slot across all presets, free + Pro.
  { id: "meter", slot: "meter", label: "Segmented LED", tier: "free", Component: Meter },
  // Alternative meter: the dot-matrix "mini EQ" spectrum (from the UI kits).
  { id: "miniEq", slot: "meter", label: "Mini EQ", tier: "free", Component: MiniEq },
];

[...freeVariants, ...proVariants].forEach(registerVariant);
