/**
 * Player config — the "skin" a user assembles (docs/component-variants-plan.md §2).
 *
 * A config is { version, preset?, variants: slot→variantId }. It is the SWAPPABLE, PERSISTED,
 * shareable data that a generic Shell resolves into rendered variants. Palette (theme × accent)
 * stays in useUiStore — orthogonal.
 *
 * VERSION-FIRST (skin-architecture.md §4): a saved config references variant ids by string. Before
 * any saved config is trusted it runs through `migratePlayerConfig`, which steps an old blob up to
 * `CONFIG_VERSION`. Renaming/removing a variant id is therefore a DATA MIGRATION across saved
 * configs — add a migrator step, never a silent rename. This ships before the picker can persist.
 */
import type { Role } from "./vocabulary";

export const CONFIG_VERSION = 1;

export interface PlayerConfig {
  version: number;
  /** Seed preset id (e.g. "porcelain" | "studio" | "aether"); per-slot overrides win over it. */
  preset?: string;
  /** Per-slot variant overrides. A slot absent here falls back to the preset, then a free default. */
  variants: Partial<Record<Role, string>>;
}

/** The free default: the Porcelain preset, no overrides. Always valid, always available. */
export function defaultPlayerConfig(): PlayerConfig {
  return { version: CONFIG_VERSION, preset: "porcelain", variants: {} };
}

/**
 * Stepwise migrators, keyed by the version they UPGRADE FROM. To bump the schema: set a new
 * `CONFIG_VERSION` and add a `MIGRATORS[oldVersion]` that returns the next-version shape. Example
 * (when a variant id is renamed): rewrite `variants` entries pointing at the old id.
 */
const MIGRATORS: Record<number, (c: PlayerConfig) => PlayerConfig> = {
  // 0: (c) => ({ ...c, version: 1, variants: renameVariant(c.variants, "old", "new") }),
};

/** Bring any persisted blob up to the current schema; falls back to the default if unusable. */
export function migratePlayerConfig(raw: unknown): PlayerConfig {
  if (!raw || typeof raw !== "object") return defaultPlayerConfig();
  let cfg = raw as PlayerConfig;
  if (typeof cfg.version !== "number" || !cfg.variants || typeof cfg.variants !== "object") {
    return defaultPlayerConfig();
  }
  let guard = 0;
  while (cfg.version < CONFIG_VERSION && guard++ < 64) {
    const step = MIGRATORS[cfg.version];
    if (!step) return defaultPlayerConfig(); // missing migrator → safe reset rather than a broken player
    cfg = step(cfg);
  }
  return cfg;
}
