import { describe, it, expect } from "vitest";
import { CONFIG_VERSION, defaultPlayerConfig, migratePlayerConfig } from "./playerConfig";

describe("playerConfig", () => {
  it("default is at the current version with no overrides", () => {
    const c = defaultPlayerConfig();
    expect(c.version).toBe(CONFIG_VERSION);
    expect(c.preset).toBe("porcelain");
    expect(c.variants).toEqual({});
  });

  it("passes a current-version config through unchanged", () => {
    const cfg = { version: CONFIG_VERSION, preset: "studio", variants: { eq: "porcelainEq" } };
    expect(migratePlayerConfig(cfg)).toEqual(cfg);
  });

  it("resets to default on unusable input", () => {
    for (const bad of [null, undefined, 42, "x", {}, { version: 1 }, { variants: {} }]) {
      expect(migratePlayerConfig(bad)).toEqual(defaultPlayerConfig());
    }
  });

  it("resets to default when a future version has no migrator path", () => {
    // version below current but no migrator registered for it → safe reset, never a broken player
    const stale = { version: -1, variants: {} };
    expect(migratePlayerConfig(stale)).toEqual(defaultPlayerConfig());
  });
});
