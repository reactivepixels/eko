/**
 * Phase 1 · Schema types — Registry / Chassis manifest / Layout / User layout.
 * See docs/skin-architecture.md §3 + §11.
 *
 * These are the shapes the Phase-2 primitives, Phase-3 renderer and Phase-7 builder all read.
 * A "player" decomposes into:
 *   - REGISTRY     — the shared pool of self-describing, feed-bound components,
 *   - CHASSIS      — a manifest (required/optional components, regions, allow-rules, default layouts),
 *   - LAYOUT       — a document placing components into regions (chassis defaults + user overrides),
 *   - PALETTE      — tokens, orthogonal (lives in useUiStore: theme × accent; not modelled here).
 */

import type { FeedId } from "./feeds";
import { REQUIRED_ROLES, type Region, type Role } from "./vocabulary";

/** A self-describing component in the shared registry (Tier 1). */
export interface RegistryEntry {
  /** Stable component id, e.g. "Knob", "TransportButtons". */
  id: string;
  label: string;
  /** The capability this component fills. */
  role: Role;
  /** Engine feeds it binds to (must align with ROLE_FEEDS for its role). */
  feeds: readonly FeedId[];
  /** Optional presentation variants, e.g. ["pills","dropdown"]. */
  variants?: readonly string[];
  /** Regions this component is allowed to occupy (default: any region the chassis allows). */
  allowedRegions?: readonly Region[];
  /** Default grid span when placed (1 = single cell). */
  defaultSpan?: number;
  /** Chassis-specific: a signature piece that does NOT travel to other chassis (§3). */
  signature?: boolean;
}

/** A registry, keyed by component id. Built in Phase 2; the contracts only need its shape. */
export type Registry = Readonly<Record<string, RegistryEntry>>;

/** One component placed into a region by a layout. */
export interface PlacedComponent {
  /** RegistryEntry.id. */
  component: string;
  /** Static props (e.g. { variant: "pills" }). */
  props?: Record<string, unknown>;
  /** Disambiguates the binding when a component's role maps to several feeds. */
  feed?: FeedId;
}

/** A layout document: which components sit in which regions. The swappable *data*. */
export type LayoutDoc = Partial<Record<Region, readonly PlacedComponent[]>>;

/** A chassis manifest (Tier 2) — what THIS chassis includes, how it's arranged, where things may go. */
export interface ChassisManifest {
  /** == useUiStore Skin id ("porcelain" | "studio" | …). Codebase "skin" == architecture "chassis". */
  id: string;
  label: string;
  /** Default palette token id this chassis ships with. */
  palette: string;
  /** Regions this chassis exposes. */
  regions: readonly Region[];
  /** Component ids that must always be present (the "must include" set). */
  required: readonly string[];
  /** Component ids the user may show/hide. */
  optional: readonly string[];
  /** Per-component placement rules — which regions each may sit in. */
  allow: Partial<Record<string, readonly Region[]>>;
  /** Named default layouts (e.g. { console, library }). */
  layouts: Record<string, LayoutDoc>;
  /** Which named layout ships as the default. */
  defaultLayout: string;
}

/** A user's customisation of a chassis (Tier 3) — persisted like the accent/skin prefs today. */
export interface UserLayout {
  chassis: string;
  /** Which named layout the user started from. */
  layout?: string;
  /** Show/hide of optional components. */
  visible?: Record<string, boolean>;
  /** Moves — component id → region (must be in the chassis's `allow` for that component). */
  placement?: Record<string, Region>;
}

// ── Pure validators (the parity rule, runnable) ───────────────────────────────

/** The set of roles a list of component ids covers, given the registry. */
export function rolesCovered(componentIds: readonly string[], registry: Registry): Set<Role> {
  const roles = new Set<Role>();
  for (const id of componentIds) {
    const entry = registry[id];
    if (entry) roles.add(entry.role);
  }
  return roles;
}

/** Required roles a manifest fails to fill. Empty ⇒ it's a complete, honest player. */
export function missingRequiredRoles(manifest: ChassisManifest, registry: Registry): Role[] {
  const present = rolesCovered([...manifest.required, ...manifest.optional], registry);
  return REQUIRED_ROLES.filter((r) => !present.has(r));
}

/** True when a user move is legal: the chassis allows that component in the target region. */
export function isPlacementAllowed(
  manifest: ChassisManifest,
  componentId: string,
  region: Region,
): boolean {
  const allowed = manifest.allow[componentId];
  return allowed ? allowed.includes(region) : false;
}
