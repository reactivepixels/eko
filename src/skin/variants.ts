/**
 * Component-variant registry (docs/component-variants-plan.md §2).
 *
 * A **variant** is a presentation component for one **slot** (a `Role` from ./vocabulary), bound
 * to that slot's headless hook and shipping its OWN material. Each skin/preset renders only its
 * own variants — the cross-skin "Customize" picker that let a config mix variants across themes
 * was REMOVED (forcing one skin's variant into another left material tokens undefined → broken
 * controls). The registry remains the lookup for a preset's own variants.
 *
 * Pure module — React types + the `Role` type only. No store, no hook, no component import, so
 * nothing can cycle through it and it is free-build safe (the free build registers only free
 * variants; Pro variants arrive via `@pro`'s `proVariants`, mirroring `proThemes`).
 *
 * Variant `id`s are a PERSISTED CONTRACT: saved configs reference them. Never rename an id without
 * a config migrator (see ./playerConfig.ts) — a silent rename drops that control from saved players.
 */
import type { ComponentType } from "react";
import type { Role } from "./vocabulary";

export type Tier = "free" | "pro";

export interface VariantDefinition {
  /** Stable id used in saved configs, e.g. "studioVolume". NEVER rename without a migrator. */
  id: string;
  /** The slot (capability) this variant fills. */
  slot: Role;
  /** Picker label, e.g. "Studio · Arc knob". */
  label: string;
  /** Gating tier. Free build only ever resolves `free` variants. */
  tier: Tier;
  /** Presentation component — consumes the slot's headless hook; owns its material. */
  Component: ComponentType;
}

const BY_ID = new Map<string, VariantDefinition>();
const BY_SLOT = new Map<Role, VariantDefinition[]>();

/** Register a variant. Throws on a duplicate id so a wiring/rename mistake fails loudly. */
export function registerVariant(def: VariantDefinition): void {
  if (BY_ID.has(def.id)) {
    throw new Error(`Variant "${def.id}" is already registered`);
  }
  BY_ID.set(def.id, def);
  const list = BY_SLOT.get(def.slot);
  if (list) list.push(def);
  else BY_SLOT.set(def.slot, [def]);
}

/** A variant by id, or undefined (e.g. a Pro variant id in a free build). */
export function getVariant(id: string | undefined): VariantDefinition | undefined {
  return id ? BY_ID.get(id) : undefined;
}

/** All registered variants for a slot, in registration order. (The cross-skin Customize picker
 *  that consumed this was removed; retained for introspection / future per-skin use.) */
export function variantsForSlot(slot: Role): VariantDefinition[] {
  return BY_SLOT.get(slot) ?? [];
}

/** Every registered variant. */
export function listVariants(): VariantDefinition[] {
  return [...BY_ID.values()];
}
